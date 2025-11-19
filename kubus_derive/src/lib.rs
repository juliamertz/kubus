use proc_macro::TokenStream;
use quote::{ToTokens, quote};
use syn::meta::ParseNestedMeta;
use syn::{FnArg, Ident, ItemFn, LitInt, LitStr, PatType, ReturnType, Type};
use syn::{parse_macro_input, parse_quote};

#[derive(PartialEq, Eq)]
enum EventType {
    Apply,
    Delete,
}

impl TryFrom<String> for EventType {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "Apply" => Ok(EventType::Apply),
            "Delete" => Ok(EventType::Delete),
            _ => Err(format!("Invalid event type {value}")),
        }
    }
}

impl ToTokens for EventType {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let ty: Type = match self {
            EventType::Apply => parse_quote!(::kubus::EventType::Apply),
            EventType::Delete => parse_quote!(::kubus::EventType::Delete),
        };
        ty.to_tokens(tokens);
    }
}

fn extract_generic_arg(ty: &Type, pos: usize) -> Option<&Type> {
    if let syn::Type::Path(type_path) = ty
        && let Some(last_segment) = type_path.path.segments.get(pos)
        && let syn::PathArguments::AngleBracketed(args) = &last_segment.arguments
        && let Some(syn::GenericArgument::Type(inner_type)) = args.args.first()
    {
        return Some(inner_type);
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
        .and_then(|ctx| extract_generic_arg(ctx, 0))
}

fn extract_function_return_error_type(func: &ItemFn) -> Option<&Type> {
    extract_return_type(func).and_then(|ty| extract_generic_arg(ty, 2))
}

fn internal_prefix(ident: Ident) -> Ident {
    let value = ident.to_string();
    let prefixed = format!("__kubus_{value}");
    Ident::new(&prefixed, ident.span())
}

fn quote_option<T>(value: Option<T>) -> proc_macro2::TokenStream
where
    T: ToTokens,
{
    match value {
        Some(inner) => quote! { Some(#inner) },
        None => quote! { None },
    }
}

#[derive(Default)]
struct EventHandlerAttrs {
    event: Option<EventType>,
    finalizer: Option<LitStr>,
    label_selector: Option<LitStr>,
    field_selector: Option<LitStr>,
    requeue_interval: Option<LitInt>,
}

impl EventHandlerAttrs {
    fn parse(&mut self, meta: ParseNestedMeta) -> syn::parse::Result<()> {
        if meta.path.is_ident("event") {
            let str: Ident = meta.value()?.parse()?;
            self.event = Some(
                str.to_string()
                    .try_into()
                    .map_err(|err| syn::parse::Error::new(str.span(), err))?,
            );
            Ok(())
        } else if meta.path.is_ident("finalizer") {
            self.finalizer = Some(meta.value()?.parse()?);
            Ok(())
        } else if meta.path.is_ident("label_selector") {
            self.label_selector = Some(meta.value()?.parse()?);
            Ok(())
        } else if meta.path.is_ident("field_selector") {
            self.field_selector = Some(meta.value()?.parse()?);
            Ok(())
        } else if meta.path.is_ident("requeue_interval") {
            self.requeue_interval = Some(meta.value()?.parse()?);
            Ok(())
        } else {
            Err(meta.error("unsupported kubus property"))
        }
    }
}

/// A procedural macro for defining Kubernetes event handlers in the Kubus framework.
///
/// # Attributes
///
/// * `event` (required) - The event type to handle: `Apply` or `Delete`
/// * `finalizer` (optional) - The finalizer name to add (for Apply) or remove (for Delete)
/// * `label_selector` (optional) - Label selector to filter which resources trigger this handler
/// * `field_selector` (optional) - Field selector to filter which resources trigger this handler
/// * `requeue_interval` (optional) - Requeue interval in seconds (default: 30)
///
/// # Function Signature
///
/// The annotated function must have the following signature:
/// ```ignore
/// async fn handler_name(
///     resource: Arc<ResourceType>,
///     context: Arc<Context<ContextType>>,
/// ) -> Result<(), ErrorType>
/// ```
///
/// # Examples
///
/// ```ignore
/// #[kubus(event = Apply, finalizer = "my-app.example.com/cleanup")]
/// async fn handle_pod_create(
///     pod: Arc<Pod>,
///     ctx: Arc<Context<MyContext>>,
/// ) -> Result<(), MyError> {
///     // Handle pod creation
///     Ok(())
/// }
///
/// #[kubus(
///     event = Delete,
///     finalizer = "my-app.example.com/cleanup",
///     requeue_interval = 60
/// )]
/// async fn handle_pod_delete(
///     pod: Arc<Pod>,
///     ctx: Arc<Context<MyContext>>,
/// ) -> Result<(), MyError> {
///     // Handle pod deletion
///     Ok(())
/// }
///
/// #[kubus(
///     event = Apply,
///     label_selector = "app=my-app",
///     field_selector = "status.phase=Running"
/// )]
/// async fn handle_running_pods(
///     pod: Arc<Pod>,
///     ctx: Arc<Context<MyContext>>,
/// ) -> Result<(), MyError> {
///     // Only handle running pods with app=my-app label
///     Ok(())
/// }
/// ```
#[proc_macro_attribute]
pub fn kubus(args: TokenStream, input: TokenStream) -> TokenStream {
    let mut attrs = EventHandlerAttrs::default();
    let attr_parser = syn::meta::parser(|meta| attrs.parse(meta));
    parse_macro_input!(args with attr_parser);

    let event = attrs.event.expect("event kubus attribute missing");

    let func = parse_macro_input!(input as ItemFn);
    let resource_ty = extract_resource_type(&func)
        .expect("unable to extract resource type from handler function");
    let context_ty =
        extract_context_type(&func).expect("unable to extract resource type from handler function");
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

    let field_selector = quote_option(attrs.field_selector);
    let label_selector = quote_option(attrs.label_selector);
    let requeue_interval = attrs.requeue_interval.unwrap_or_else(|| parse_quote!(30));

    let update_finalizer = attrs
        .finalizer
        .map(|finalizer| {
            let update_func = match event {
                EventType::Apply => quote! { ::kubus::apply_finalizer },
                EventType::Delete => quote! { ::kubus::remove_finalizer },
            };
            quote! {
                let namespace = resource.namespace();
                let client = context.client.clone();
                let api: ::kube::Api<#resource_ty> =
                    <#resource_ty as ::kube::Resource>::Scope::api(client, namespace);

                #update_func(&api, #finalizer, resource).await?;
            }
        })
        .unwrap_or_default();

    let handler_name = LitStr::new(&struct_name.to_string(), struct_name.span());

    quote! {
        #[allow(non_snake_case)]
        #internal_func

        #[allow(non_camel_case_types)]
        #[doc(hidden)]
        pub struct #struct_name;

        #[::async_trait::async_trait]
        impl ::kubus::Handler<#resource_ty, #context_ty, #error_ty> for #struct_name {
            const NAME: &'static str = #handler_name;
            const FIELD_SELECTOR: Option<&'static str> = #field_selector;
            const LABEL_SELECTOR: Option<&'static str> = #label_selector;

            async fn handle(
                resource: ::std::sync::Arc<#resource_ty>,
                context: ::std::sync::Arc<::kubus::Context<#context_ty>>,
            ) -> ::std::result::Result<::kube::runtime::controller::Action, #error_ty> {
                use ::kubus::ScopeExt;
                use ::kube::{Resource, ResourceExt};
                let requeue = ::kube::runtime::controller::Action::requeue(
                    ::std::time::Duration::from_secs(#requeue_interval)
                );

                if let (::kubus::EventType::Apply, Some(_)) | (::kubus::EventType::Delete, None) =
                    (#event, resource.meta().deletion_timestamp.as_ref())
                {
                    return Ok(requeue);
                }

                #internal_func_name(resource.clone(), context.clone())
                    .await
                    .map_err(|err| ::kubus::Error::Handler(Box::new(err)))?;

                #update_finalizer

                Ok(requeue)
            }
        }

        #[::async_trait::async_trait]
        impl ::kubus::Runnable<#context_ty, #error_ty> for #struct_name {
            fn name(&self) -> &'static str {
                <#struct_name as ::kubus::Handler<#resource_ty, #context_ty, #error_ty>>::NAME
            }

            async fn run(
                &self,
                client: ::kube::Client,
                context: ::std::sync::Arc<::kubus::Context<#context_ty>>,
            ) -> ::std::result::Result<(), #error_ty> {
                <#struct_name as ::kubus::Handler<#resource_ty, #context_ty, #error_ty>>::run(self, client, context).await
            }
        }
    }
    .into()
}

enum AdmissionKind {
    Validating,
    Mutating,
}

#[derive(Default)]
struct AdmissionHandlerAttrs {
    kind: Option<AdmissionKind>,
}

impl AdmissionHandlerAttrs {
    fn parse(&mut self, meta: ParseNestedMeta) -> syn::parse::Result<()> {
        if meta.path.is_ident("validating") && self.kind.is_none() {
            self.kind = Some(AdmissionKind::Validating);
            Ok(())
        } else if meta.path.is_ident("mutating") && self.kind.is_none() {
            self.kind = Some(AdmissionKind::Mutating);
            Ok(())
        } else {
            Err(meta.error("unsupported kubus property"))
        }
    }
}

#[proc_macro_attribute]
pub fn admission(args: TokenStream, input: TokenStream) -> TokenStream {
    let mut attrs = AdmissionHandlerAttrs::default();
    let attr_parser = syn::meta::parser(|meta| attrs.parse(meta));
    parse_macro_input!(args with attr_parser);

    let kind = attrs.kind.expect("admission attribute must specify either 'mutating' or 'validating'");

    let func = parse_macro_input!(input as ItemFn);
    let func_name = &func.sig.ident;
    let name_string = LitStr::new(&func_name.to_string(), func_name.span());

    // Extract error type from return type
    let error_ty = extract_function_return_error_type(&func)
        .cloned()
        .unwrap_or_else(|| parse_quote! { ::kubus::HandlerError });

    let internal_func = {
        let mut func = func.clone();
        func.sig.ident = internal_prefix(func.sig.ident);
        func
    };
    let internal_func_name = &internal_func.sig.ident;

    let (trait_name, method_name): (Type, Ident) = match kind {
        AdmissionKind::Mutating => (
            parse_quote! { ::kubus::admission::MutatingAdmissionHandler },
            Ident::new("mutate", func_name.span()),
        ),
        AdmissionKind::Validating => (
            parse_quote! { ::kubus::admission::ValidatingAdmissionHandler },
            Ident::new("validate", func_name.span()),
        ),
    };

    quote! {
        #[allow(non_snake_case)]
        #internal_func

        #[allow(non_camel_case_types)]
        #[doc(hidden)]
        pub struct #func_name;

        #[::async_trait::async_trait]
        impl #trait_name for #func_name {
            type Err = #error_ty;

            fn name(&self) -> &'static str {
                #name_string
            }

            async fn #method_name(
                &self,
                req: &::kube::core::admission::AdmissionRequest<::kube::api::DynamicObject>,
            ) -> ::std::result::Result<::kube::core::admission::AdmissionResponse, Self::Err> {
                #internal_func_name(req).await
            }
        }
    }
    .into()
}
