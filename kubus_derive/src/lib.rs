use proc_macro::TokenStream;
use quote::quote;
use syn::{Ident, ItemFn, LitStr, meta::ParseNestedMeta, parse_macro_input};

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

fn extract_resource_type(func: &ItemFn) -> proc_macro2::TokenStream {
    if let Some(arg) = func.sig.inputs.iter().nth(1)
        && let syn::FnArg::Typed(pat_type) = arg
        && let syn::Type::Path(type_path) = &*pat_type.ty
    {
        return quote! { #type_path };
    }
    panic!("Function must have a resource parameter");
}

fn parse_event_type(event_name: Ident) -> proc_macro2::TokenStream {
    match event_name.to_string().as_str() {
        "Apply" => quote! { kube::runtime::watcher::Event::Apply(resource) },
        "InitApply" => quote! { kube::runtime::watcher::Event::InitApply(resource) },
        "Delete" => quote! { kube::runtime::watcher::Event::Delete(resource) },
        _ => unimplemented!(),
    }
}

#[proc_macro_attribute]
pub fn kubus(args: TokenStream, input: TokenStream) -> TokenStream {
    let mut attrs = Attrs::default();
    let attr_parser = syn::meta::parser(|meta| attrs.parse(meta));
    parse_macro_input!(args with attr_parser);

    let input = parse_macro_input!(input as ItemFn);
    let resource_arg_type = extract_resource_type(&input);
    let event_type = parse_event_type(attrs.event.expect("event kubus attribute missing"));
    let fn_name = &input.sig.ident;
    let struct_name = Ident::new(&format!("__Kubus_{}", fn_name), fn_name.span());

    let finalizer = if let Some(finalizer) = attrs.finalizer {
        quote! { Some(#finalizer) }
    } else {
        quote! { None }
    };

    quote! {
        #[inline]
        #input

        #[allow(non_camel_case_types)]
        #[doc(hidden)]
        pub struct #struct_name;

        impl #struct_name {
            pub async fn handler(
                client: ::kubus::kube::Client,
                resource: #resource_arg_type
            ) -> ::kubus::Result<()> {
                #fn_name(client, resource)
            }

            pub async fn watch(client: kube::Client) -> ::kubus::Result<()> {
                use futures::StreamExt;
                use kube::runtime::watcher::{self, Event};

                let api = kube::Api::<#resource_arg_type>::all(client.clone());
                let mut stream = watcher::watcher(api.clone(), watcher::Config::default()).boxed();

                while let Some(event) = stream.next().await {
                    match event {
                        Ok(#event_type) => {
                            if let Err(e) = Self::handler(client.clone(), resource.clone()).await {
                                tracing::error!("handler {} failed: {}", stringify!(#fn_name), e);
                            } else {
                                if let Some(finalizer_name) = #finalizer {
                                    let namespace = resource
                                        .namespace()
                                        .to_owned()
                                        .unwrap_or_else(|| "default".into());
                                    let api = ::kubus::kube::Api::namespaced(client.clone(), &namespace);
                                    ::kubus::update_finalizer(
                                        &api,
                                        finalizer_name,
                                        std::sync::Arc::new(resource)
                                    ).await?;
                                }
                            }
                        }
                        Ok(_) => {},
                        Err(e) => {
                            tracing::error!("watch error in {}: {}", stringify!(#fn_name), e)
                        },
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
