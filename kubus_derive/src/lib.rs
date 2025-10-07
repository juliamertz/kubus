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

fn extract_resource_type(func: &ItemFn) -> proc_macro2::TokenStream {
    if let Some(arg) = func.sig.inputs.first()
        && let syn::FnArg::Typed(pat_type) = arg
        && let syn::Type::Path(type_path) = &*pat_type.ty
    {
        if let Some(last_segment) = type_path.path.segments.last()
            && last_segment.ident == "Arc"
            && let syn::PathArguments::AngleBracketed(args) = &last_segment.arguments
            && let Some(syn::GenericArgument::Type(inner_type)) = args.args.first()
        {
            return quote! { #inner_type };
        }
        return quote! { #type_path };
    }
    panic!("Function must have a resource parameter");
}

fn get_event_type(event_name: Ident) -> proc_macro2::TokenStream {
    match event_name.to_string().as_str() {
        "Apply" => quote! { ::kubus::EventType::Apply },
        "InitApply" => quote! { ::kubus::EventType::InitApply },
        "Delete" => quote! { ::kubus::EventType::Delete },
        _ => unimplemented!(),
    }
}

#[proc_macro_attribute]
pub fn kubus(args: TokenStream, input: TokenStream) -> TokenStream {
    let mut attrs = Attrs::default();
    let attr_parser = syn::meta::parser(|meta| attrs.parse(meta));
    parse_macro_input!(args with attr_parser);

    let input = parse_macro_input!(input as ItemFn);
    let resource_ty = extract_resource_type(&input);
    let event_type = get_event_type(attrs.event.expect("event kubus attribute missing"));

    let fn_name = &input.sig.ident;
    let struct_name = Ident::new(&format!("__Kubus_{}", fn_name), fn_name.span());

    quote! {
        #[inline]
        #input

        #[allow(non_camel_case_types)]
        #[doc(hidden)]
        pub struct #struct_name;

        impl #struct_name {
            pub async fn handler(
                resource: ::std::sync::Arc<#resource_ty>,
                context: ::std::sync::Arc<::kubus::Context>
            ) -> ::kubus::Result<::kubus::kube::runtime::controller::Action> {
                use ::kubus::kube::{Resource, ResourceExt};
                if let (::kubus::EventType::Apply, Some(_)) | (::kubus::EventType::Delete, None) =
                    (#event_type, resource.meta().deletion_timestamp.as_ref())
                {
                    return Ok(::kubus::kube::runtime::controller::Action::requeue(Duration::from_secs(60)));
                }

                #fn_name(resource, context).await
            }

            pub fn error_policy(
                resource: ::std::sync::Arc<#resource_ty>,
                err: &::kubus::Error,
                ctx: Arc<::kubus::Context>
            ) -> ::kubus::kube::runtime::controller::Action {
                tracing::warn!("Reconciliation error: {:?}", err);

                ::kubus::kube::runtime::controller::Action::requeue(Duration::from_secs(60))
            }


            pub async fn watch(client: kube::Client) -> ::kubus::Result<()> {
                use futures::StreamExt;
                use kube::runtime::watcher::{self, Event, Config};
                use kube::{Resource, ResourceExt};
                use kube::runtime::controller::Controller;

                let api = ::kubus::kube::Api::<#resource_ty>::all(client.clone());
                let context = ::kubus::Context { client };
                let controller = Controller::new(api, Config::default())
                    .shutdown_on_signal()
                    .run(Self::handler, Self::error_policy, ::std::sync::Arc::new(context))
                    .filter_map(|x| async move { std::result::Result::ok(x) })
                    .for_each(|_| futures::future::ready(()));

                // TODO: why is this needed?
                tokio::select! {
                    _ = controller => {
                        tracing::info!("controller finished");
                        ::std::process::exit(1);
                    }
                    _ = tokio::signal::ctrl_c() => {
                        tracing::info!("received shutdown signal");
                    }
                }

                Ok(())
            }
        }

        ::kubus::inventory::submit! {
            ::kubus::Handler {
                name: stringify!(#fn_name),
                watch_fn: |client| Box::pin(#struct_name::watch(client)),
            }
        }
    }
    .into()
}
