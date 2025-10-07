use proc_macro::TokenStream;
use quote::quote;
use syn::{Ident, ItemFn, LitStr, Type, meta::ParseNestedMeta, parse_macro_input};

#[derive(Default)]
struct Attrs {
    kind: Option<Type>,
    event: Option<Ident>,
    finalizer: Option<LitStr>,
}

impl Attrs {
    fn parse(&mut self, meta: ParseNestedMeta) -> syn::parse::Result<()> {
        if meta.path.is_ident("event") {
            self.event = Some(meta.value()?.parse()?);
            Ok(())
        } else if meta.path.is_ident("kind") {
            self.kind = Some(meta.value()?.parse()?);
            Ok(())
        } else if meta.path.is_ident("finalizer") {
            self.finalizer = Some(meta.value()?.parse()?);
            Ok(())
        } else {
            Err(meta.error("unsupported kubus property"))
        }
    }
}

fn extract_inner_arc(ty: &Type) -> proc_macro2::TokenStream {
    if let syn::Type::Path(type_path) = ty {
        if let Some(last_segment) = type_path.path.segments.last()
            && last_segment.ident == "Arc"
            && let syn::PathArguments::AngleBracketed(args) = &last_segment.arguments
            && let Some(syn::GenericArgument::Type(inner_type)) = args.args.first()
        {
            return quote! { #inner_type };
        }
        return quote! { #type_path };
    }
    panic!("Cannot extract from type as Arc");
}

fn extract_resource_type(func: &ItemFn) -> proc_macro2::TokenStream {
    if let Some(arg) = func.sig.inputs.first()
        && let syn::FnArg::Typed(pat_type) = arg
    {
        extract_inner_arc(&pat_type.ty)
    } else {
        panic!("Function must have a resource parameter");
    }
}

fn extract_context_type(func: &ItemFn) -> proc_macro2::TokenStream {
    if let Some(arg) = func.sig.inputs.iter().nth(1)
        && let syn::FnArg::Typed(pat_type) = arg
    {
        extract_inner_arc(&pat_type.ty)
    } else {
        panic!("Function must have a resource parameter");
    }
}

fn get_event_type(event_name: Ident) -> proc_macro2::TokenStream {
    match event_name.to_string().as_str() {
        "Apply" => quote! { ::kubus::EventType::Apply },
        "InitApply" => quote! { ::kubus::EventType::InitApply },
        "Delete" => quote! { ::kubus::EventType::Delete },
        _ => unimplemented!(),
    }
}

fn internal_prefix(ident: Ident) -> Ident {
    let value = ident.to_string();
    let prefixed = format!("__Kubus_{value}");
    Ident::new(&prefixed, ident.span())
}

#[proc_macro_attribute]
pub fn kubus(args: TokenStream, input: TokenStream) -> TokenStream {
    let mut attrs = Attrs::default();
    let attr_parser = syn::meta::parser(|meta| attrs.parse(meta));
    parse_macro_input!(args with attr_parser);

    let func = parse_macro_input!(input as ItemFn);
    let resource_ty = extract_resource_type(&func);
    let context_ty = extract_context_type(&func);
    let event_type = get_event_type(attrs.event.expect("event kubus attribute missing"));

    let internal_func = {
        let mut func = func.clone();
        func.sig.ident = internal_prefix(func.sig.ident);
        func
    };

    let struct_name = func.sig.ident;
    let internal_func_name = internal_func.sig.ident.clone();

    let finalizer = attrs
        .finalizer
        .map(|name| quote! { Some(#name) })
        .unwrap_or_else(|| quote! { None });

    quote! {
        #[allow(non_snake_case)]
        #internal_func

        #[allow(non_camel_case_types)]
        #[doc(hidden)]
        pub struct #struct_name;

        impl ::kubus::EventHandler<#resource_ty, #context_ty> for #struct_name {
            fn handler<'async_trait>(
                resource: Arc<#resource_ty>,
                context: Arc<Context>,
            ) -> ::core::pin::Pin<
                Box<
                    dyn ::core::future::Future<Output = Result<Action>>
                        + ::core::marker::Send
                        + 'async_trait,
                >,
            > {
                Box::pin(async move {
                    if let ::core::option::Option::Some(__ret) =
                        ::core::option::Option::None::<Result<Action>>
                    {
                        #[allow(unreachable_code)]
                        return __ret;
                    }
                    let resource = resource;
                    let context = context;
                    let __ret: Result<Action> = {
                        use ::kubus::ScopeExt;
                        use ::kubus::kube::{Resource, ResourceExt};

                        if let (::kubus::EventType::Apply, Some(_)) | (::kubus::EventType::Delete, None) =
                            (#event_type, resource.meta().deletion_timestamp.as_ref())
                        {
                            return Ok(::kubus::kube::runtime::controller::Action::requeue(Duration::from_secs(15)));
                        }

                        #internal_func_name(resource.clone(), context).await?;

                        // TODO: store shared client in context wrapper
                        let client = ::kubus::kube::Client::try_default().await?;

                        if let Some(finalizer) = #finalizer {
                            let namespace = resource.namespace();
                            let api: kube::Api<Pod> =
                                <#resource_ty as ::kubus::kube::Resource>::Scope::api(client, namespace);

                            ::kubus::update_finalizer(
                                &api,
                                &finalizer,
                                #event_type,
                                resource
                            ).await?;
                        }

                        Ok(Action::requeue(::std::time::Duration::from_secs(15)))
                    };
                    #[allow(unreachable_code)]
                    __ret
                })
            }
        }
    }
    .into()
}
