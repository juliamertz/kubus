use proc_macro::TokenStream;
use quote::quote;
use syn::{
    FnArg, Ident, ItemFn, LitStr, PatType, ReturnType, Type, meta::ParseNestedMeta,
    parse_macro_input, parse_quote,
};

#[derive(Default)]
struct Attrs {
    event: Option<Ident>,
    finalizer: Option<LitStr>,
}

impl Attrs {
    fn parse(&mut self, meta: ParseNestedMeta) -> syn::parse::Result<()> {
        if meta.path.is_ident("event") {
            self.event = Some(meta.value()?.parse()?);
            Ok(())
        } else if meta.path.is_ident("finalizer") {
            self.finalizer = Some(meta.value()?.parse()?);
            Ok(())
        } else {
            Err(meta.error("unsupported kubus property"))
        }
    }
}

fn extract_generic_arg(ty: &Type, pos: usize) -> Option<&Type> {
    if let syn::Type::Path(type_path) = ty {
        if let Some(last_segment) = type_path.path.segments.get(pos)
            && let syn::PathArguments::AngleBracketed(args) = &last_segment.arguments
            && let Some(syn::GenericArgument::Type(inner_type)) = args.args.first()
        {
            return Some(inner_type);
        }
    }
    None
}

fn extract_return_type(func: &ItemFn) -> Option<&Type> {
    match func.sig.output {
        ReturnType::Type(_, ref ty) => Some(ty),
        ReturnType::Default => None,
    }
}

fn extract_func_arg_type(arg: &FnArg) -> Option<&PatType> {
    match arg {
        FnArg::Typed(pat) => Some(pat),
        FnArg::Receiver(_) => None,
    }
}

fn extract_resource_type(func: &ItemFn) -> Option<&Type> {
    func.sig
        .inputs
        .first()
        .and_then(|arg| extract_func_arg_type(arg))
        .and_then(|pat| extract_generic_arg(&pat.ty, 0))
}

fn extract_context_type(func: &ItemFn) -> Option<&Type> {
    func.sig
        .inputs
        .iter()
        .nth(1)
        .and_then(|arg| extract_func_arg_type(arg))
        .and_then(|arc| extract_generic_arg(&arc.ty, 0))
        .and_then(|ctx| extract_generic_arg(&ctx, 0))
}

fn extract_function_return_error_type(func: &ItemFn) -> Option<&Type> {
    extract_return_type(func).and_then(|ty| extract_generic_arg(ty, 2))
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
    let prefixed = format!("__kubus_{value}");
    Ident::new(&prefixed, ident.span())
}

#[proc_macro_attribute]
pub fn kubus(args: TokenStream, input: TokenStream) -> TokenStream {
    let mut attrs = Attrs::default();
    let attr_parser = syn::meta::parser(|meta| attrs.parse(meta));
    parse_macro_input!(args with attr_parser);

    let func = parse_macro_input!(input as ItemFn);
    let resource_ty = extract_resource_type(&func)
        .expect("unable to extract resource type from handler function");
    let context_ty =
        extract_context_type(&func).expect("unable to extract resource type from handler function");
    let event_type = get_event_type(attrs.event.expect("event kubus attribute missing"));
    let error_ty = extract_function_return_error_type(&func)
        .cloned()
        .unwrap_or_else(|| parse_quote! { ::kubus::HandlerError });

    let internal_func = {
        let mut func = func.clone();
        func.sig.ident = internal_prefix(func.sig.ident);
        func
    };

    let struct_name = func.sig.ident.clone();
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

        // TODO: supply custom error type with attrs
        impl ::kubus::EventHandler<#resource_ty, #context_ty, #error_ty> for #struct_name {
            fn handler<'async_trait>(
                resource: ::std::sync::Arc<#resource_ty>,
                context: ::std::sync::Arc<::kubus::Context<#context_ty>>,
            ) -> ::core::pin::Pin<
                Box<
                    dyn ::core::future::Future<Output = ::std::result::Result<::kube::runtime::controller::Action, #error_ty>>
                        + ::core::marker::Send
                        + 'async_trait,
                >,
            > {
                Box::pin(async move {
                    if let ::core::option::Option::Some(__ret) =
                        ::core::option::Option::None::<::std::result::Result<::kube::runtime::controller::Action, #error_ty>>
                    {
                        #[allow(unreachable_code)]
                        return __ret;
                    }
                    let resource = resource;
                    let context = context;

                    let __ret: ::std::result::Result<::kube::runtime::controller::Action, #error_ty> = {
                        use ::kubus::ScopeExt;
                        use ::kube::{Resource, ResourceExt};

                        if let (::kubus::EventType::Apply, Some(_)) | (::kubus::EventType::Delete, None) =
                            (#event_type, resource.meta().deletion_timestamp.as_ref())
                        {
                            return Ok(::kube::runtime::controller::Action::requeue(Duration::from_secs(15)));
                        }

                        #internal_func_name(resource.clone(), context.clone())
                            .await
                            .map_err(|err| ::kubus::Error::Handler(Box::new(err)))?;

                        if let Some(finalizer) = #finalizer {
                            let namespace = resource.namespace();
                            let client = context.client.clone();
                            let api: ::kube::Api<#resource_ty> =
                                <#resource_ty as ::kube::Resource>::Scope::api(client, namespace);

                            ::kubus::update_finalizer(
                                &api,
                                &finalizer,
                                #event_type,
                                resource
                            ).await?;
                        }

                        Ok(::kube::runtime::controller::Action::requeue(::std::time::Duration::from_secs(15)))
                    };
                    #[allow(unreachable_code)]
                    __ret
                })
            }
        }
    }
    .into()
}
