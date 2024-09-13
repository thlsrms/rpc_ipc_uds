use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};

use proc_macro::TokenStream;
use quote::quote;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{parse_macro_input, DeriveInput, Fields, LitStr, Meta, Token};

#[proc_macro_derive(RpcService, attributes(rpc_service, response, client))]
pub fn rpc_service_derive(input: TokenStream) -> TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);
    service_impl(ast)
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

fn service_impl(ast: DeriveInput) -> Result<proc_macro2::TokenStream, syn::Error> {
    let name = ast.ident;

    let variants = match &ast.data {
        syn::Data::Enum(data_enum) => &data_enum.variants,
        _ => panic!("#[derive(RpcService)] is only valid for enums"),
    };

    let request_enum_ident = new_ident("Req");
    let response_enum_ident = new_ident("Res");
    let packet_ident = new_ident("Message");

    let serialization_option = extract_serialization_option(&ast.attrs)?;
    let rpc_structs = generate_rpc_request_response(&name, serialization_option.as_deref());
    let (rpc_variants, payload_enums) =
        generate_payload_enums(&name, variants, serialization_option.as_deref())?;

    let mut service_methods = vec![];
    for (variant, v_kind) in &rpc_variants {
        let variant_name = &variant.ident;
        let (method_name, ret_tokens) = if v_kind == "service" {
            (
                quote::format_ident!(
                    "handle_{}_request",
                    to_snake_case(&variant_name.to_string())
                ),
                quote! { -> #response_enum_ident; },
            )
        } else {
            (
                quote::format_ident!("on_{}_response", to_snake_case(&variant_name.to_string())),
                quote! { ; },
            )
        };

        match &variant.fields {
            Fields::Unnamed(fields) => {
                let args = fields.unnamed.iter().enumerate().map(|(i, f)| {
                    let ident = quote::format_ident!("arg{}", i);
                    let ty = &f.ty;
                    quote! { #ident: #ty }
                });

                service_methods.push(quote! {
                    async fn #method_name(&self, #(#args),*) #ret_tokens
                });
            }
            Fields::Named(fields) => {
                let args = fields.named.iter().map(|f| {
                    let ident = f.ident.as_ref().unwrap();
                    let ty = &f.ty;
                    quote! { #ident: #ty }
                });

                service_methods.push(quote! {
                    async fn #method_name(&self, #(#args),*) #ret_tokens
                });
            }
            Fields::Unit => {
                service_methods.push(quote! {
                    async fn #method_name(&self) #ret_tokens
                });
            }
        }
    }

    let service_trait_ident = quote::format_ident!("{}Service", name);
    let service_impl = quote! {
        #rpc_structs

        #payload_enums

        pub trait #service_trait_ident {
            async fn send_client_message(&self, message: #packet_ident) -> Result<(), Box<dyn std::error::Error>>;
            async fn on_request_payload(&mut self, payload: #request_enum_ident) -> #response_enum_ident;
            async fn on_response_payload(&mut self, payload: #response_enum_ident);
            #(#service_methods)*
        }
    };

    Ok(service_impl)
}

#[proc_macro_derive(RpcClient, attributes(rpc_client, response, client))]
pub fn rpc_client_derive(input: TokenStream) -> TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);

    client_impl(ast)
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

fn client_impl(ast: DeriveInput) -> Result<proc_macro2::TokenStream, syn::Error> {
    let name = ast.ident;

    let variants = match &ast.data {
        syn::Data::Enum(data_enum) => &data_enum.variants,
        _ => panic!("#[derive(RpcClient)] is only valid for enums"),
    };

    let packet_ident = new_ident("Message");
    let payload_enum_ident = new_ident("Payload");
    let request_enum_ident = new_ident("Req");
    let response_enum_ident = new_ident("Res");
    let payload_origin_ident = new_ident("Origin");

    let serialization_option = extract_serialization_option(&ast.attrs)?;
    let rpc_structs = generate_rpc_request_response(&name, serialization_option.as_deref());
    let (rpc_variants, payload_enums) =
        generate_payload_enums(&name, variants, serialization_option.as_deref())?;

    let mut client_methods = vec![];
    for (variant, v_kind) in rpc_variants {
        let variant_name = &variant.ident;
        let method_name = if v_kind == "service" {
            to_snake_case(&variant_name.to_string())
        } else {
            quote::format_ident!(
                "handle_{}_request",
                to_snake_case(&variant_name.to_string())
            )
        };

        match &variant.fields {
            Fields::Unnamed(fields) => {
                let args = fields.unnamed.iter().enumerate().map(|(i, f)| {
                    let ident = quote::format_ident!("arg{}", i);
                    let ty = &f.ty;
                    (quote! { #ident: #ty }, quote! { #ident })
                });
                let (args, args_for_call): (Vec<_>, Vec<_>) = args.unzip();

                if v_kind == "service" {
                    client_methods.push(quote! {
                    fn #method_name(&self, #(#args),*) -> Result<(), Box<dyn std::error::Error>> {
                        self.send_message(#packet_ident {
                            id: self.generate_request_id(),
                            origin: #payload_origin_ident::Client,
                            payload: #payload_enum_ident::Request(#request_enum_ident::#variant_name( #(#args_for_call),* )),
                        })
                    }
                });
                } else {
                    client_methods.push(quote! {
                        fn #method_name(&self, #(#args),*) -> #response_enum_ident;
                    })
                }
            }
            Fields::Named(fields) => {
                let args = fields.named.iter().map(|f| {
                    let ident = f.ident.as_ref().unwrap();
                    let ty = &f.ty;
                    (quote! { #ident: #ty }, quote! { #ident })
                });
                let (args, args_for_call): (Vec<_>, Vec<_>) = args.unzip();

                if v_kind == "service" {
                    client_methods.push(quote! {
                    fn #method_name(&self, #(#args),*) -> Result<(), Box<dyn std::error::Error>> {
                        self.send_message(#packet_ident {
                            id: self.generate_request_id(),
                            origin: #payload_origin_ident::Client,
                            payload: #payload_enum_ident::Request(#request_enum_ident::#variant_name { #(#args_for_call),* }),
                        } )
                    }
                });
                } else {
                    client_methods.push(quote! {
                        fn #method_name(&self, #(#args),*) -> #response_enum_ident;
                    })
                }
            }
            Fields::Unit => {
                if v_kind == "service" {
                    client_methods.push(quote! {
                    fn #method_name(&self) -> Result<(), Box<dyn std::error::Error>> {
                        self.send_message(#packet_ident {
                            id: self.generate_request_id(),
                            origin: #payload_origin_ident::Client,
                            payload: #payload_enum_ident::Request(#request_enum_ident::#variant_name),
                        })
                    }
                });
                } else {
                    client_methods.push(quote! {
                        fn #method_name(&self) -> #response_enum_ident;
                    })
                }
            }
        }
    }

    let client_trait_ident = quote::format_ident!("{}Client", name);
    let client_impl = quote! {
        #rpc_structs

        #payload_enums

        pub trait #client_trait_ident {
            fn send_message(&self, request: #packet_ident) -> Result<(), Box<dyn std::error::Error>>;
            fn generate_request_id(&self) -> u32;

            #(#client_methods)*
        }
    };

    Ok(client_impl)
}

fn to_snake_case(s: &str) -> proc_macro2::Ident {
    let mut snake = String::new();
    for (i, ch) in s.chars().enumerate() {
        if ch.is_uppercase() && i != 0 {
            snake.push('_');
        }
        snake.push(ch.to_ascii_lowercase());
    }
    proc_macro2::Ident::new(&snake, proc_macro2::Span::call_site())
}

// Static hashset to ensure generated types are only generated once.
static GENERATED_TYPES: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

type FilteredVariant<'v> = (&'v syn::Variant, String);

fn generate_payload_enums<'a>(
    name: &syn::Ident,
    variants: &'a Punctuated<syn::Variant, Token![,]>,
    serialization: Option<&str>,
) -> Result<(Vec<FilteredVariant<'a>>, proc_macro2::TokenStream), syn::Error> {
    let payload_enum_ident = new_ident("Payload");
    let request_enum_ident = new_ident("Req");
    let response_enum_ident = new_ident("Res");

    let mut responses_map: HashMap<String, String> = HashMap::new();
    let mut response_variants_map: HashMap<String, &syn::Variant> = HashMap::new();
    let mut filtered_rpcs: Vec<(&syn::Variant, String)> = vec![];
    let mut request_rpcs: Vec<&syn::Variant> = vec![];

    for variant in variants {
        let variant_name = &variant.ident;
        let variant_attrs = &variant.attrs;
        let variant_kind = extract_variant_attributes(variant_attrs)?;
        match variant_kind {
            VariantKind::ServiceRpc => {
                filtered_rpcs.push((variant, "service".into()));
                request_rpcs.push(variant);
            }
            VariantKind::ClientRpc => {
                filtered_rpcs.push((variant, "client".into()));
                request_rpcs.push(variant);
            }
            VariantKind::Response(response_to) => {
                // Treat this variant as response if marked with the [response(variants)] attribute
                // Use the variant names to identify which variant uses this one as response
                response_variants_map.insert(variant_name.to_string(), variant);
                for rpc in response_to.into_iter() {
                    responses_map.insert(rpc, variant_name.to_string());
                }
            }
        }
    }

    // Check if we've already generated this
    let mut generated = GENERATED_TYPES.lock().unwrap();
    if generated.contains(&request_enum_ident.to_string()) {
        return Ok((filtered_rpcs, proc_macro2::TokenStream::new()));
    }

    let mut response_variants = vec![];
    for variant in &request_rpcs {
        let variant_name = &variant.ident;
        let response_variant = if let Some(res) = responses_map.get(&variant_name.to_string()) {
            response_variants_map.get(res).unwrap()
        } else {
            variant
        };

        response_variants.push(new_enum_variant_from(
            variant_name,
            &response_variant.fields,
        ));
    }

    let serialization_derives = serialization_derives(serialization);

    let error_name = quote::format_ident!("{}Error", name);
    let request_rpcs_variants: Vec<proc_macro2::TokenStream> = request_rpcs
        .iter()
        .map(|v| new_enum_variant_from(&v.ident, &v.fields))
        .collect();
    let request_enum = quote! {
        pub enum #request_enum_ident {
            #(#request_rpcs_variants),*,
        }
    };

    let response_enum = quote! {
        pub enum #response_enum_ident {
            #(#response_variants),*,
            Error(#error_name),
        }
    };

    generated.insert(request_enum_ident.to_string());
    generated.insert(response_enum_ident.to_string());
    Ok((
        filtered_rpcs,
        quote! {
            #serialization_derives
            #request_enum

            #serialization_derives
            #response_enum

            #serialization_derives
            pub enum #payload_enum_ident {
                Request(#request_enum_ident),
                Response(#response_enum_ident),
            }
        },
    ))
}

fn generate_rpc_request_response(
    name: &syn::Ident,
    serialization: Option<&str>,
) -> proc_macro2::TokenStream {
    let packet_ident = new_ident("Message");
    let payload_enum_ident = new_ident("Payload");
    let payload_origin_ident = new_ident("Origin");
    let error_kind_ident = quote::format_ident!("{}ErrorKind", name);
    let error_ident = quote::format_ident!("{}Error", name);

    // Check if we've already generated this
    let mut generated = GENERATED_TYPES.lock().unwrap();
    if generated.contains(&packet_ident.to_string()) {
        return proc_macro2::TokenStream::new();
    }

    let serialization_derives = serialization_derives(serialization);

    generated.insert(packet_ident.to_string());
    quote! {
        #serialization_derives
        pub struct #packet_ident {
            pub id: u32,
            pub origin: #payload_origin_ident,
            pub payload: #payload_enum_ident,
        }

        #serialization_derives
        #[derive(PartialEq)]
        pub enum #payload_origin_ident {
            Service,
            Client,
        }

        #serialization_derives
        pub enum #error_kind_ident {
            Timeout,
            InvalidArgument,
            PermissionDenied,
            NotFound,
            Unimplemented,
            Unreachable,
            Internal,
        }

        impl std::fmt::Display for #error_kind_ident {
            fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                match self {
                    Self::Timeout => write!(f, "Timeout"),
                    Self::InvalidArgument => write!(f, "InvalidArgument"),
                    Self::PermissionDenied => write!(f, "PermissionDenied"),
                    Self::NotFound => write!(f, "NotFound"),
                    Self::Unimplemented => write!(f, "Unimplemented"),
                    Self::Unreachable => write!(f, "Unreachable"),
                    Self::Internal => write!(f, "Internal"),
                }
            }
        }

        #serialization_derives
        pub struct #error_ident {
            error: #error_kind_ident,
            description: String,
        }

        impl #error_ident {
            pub fn new(error: #error_kind_ident) -> Self {
                Self {
                    description: error.to_string(),
                    error,
                }
            }

            pub fn set_description(&mut self, description: impl Into<String>) {
                self.description = description.into();
            }
        }

        impl std::fmt::Display for #error_ident {
            fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(f, "{}", self.error)
            }
        }

        impl std::error::Error for #error_ident {
            fn description(&self) -> &str {
                &self.description
            }
        }
    }
}

fn new_ident(name: &str) -> proc_macro2::Ident {
    proc_macro2::Ident::new(name, proc_macro2::Span::call_site())
}

fn new_enum_variant_from(
    variant_name: &syn::Ident,
    fields: &syn::Fields,
) -> proc_macro2::TokenStream {
    match fields {
        Fields::Unnamed(fields) => {
            let args = fields.unnamed.iter().map(|f| {
                let ty = &f.ty;
                quote! { #ty }
            });

            quote! { #variant_name(#(#args ),*)}
        }
        Fields::Named(fields) => {
            let args = fields.named.iter().map(|f| {
                let ident = f.ident.as_ref().unwrap();
                let ty = &f.ty;
                quote! { #ident: #ty }
            });

            quote! { #variant_name { #(#args ),* }}
        }
        Fields::Unit => {
            quote! { #variant_name }
        }
    }
}

fn serialization_derives(option: Option<&str>) -> proc_macro2::TokenStream {
    match option {
        Some("bincode") => quote! {
            #[derive(::bincode::Encode, ::bincode::Decode, Debug, Clone)]
        },
        Some("serde") => quote! {
            #[derive(::serde::Serialize, ::serde::Deserialize, Debug, Clone)]
        },
        _ => quote! {
            #[derive(Debug, Clone)]
        },
    }
}

fn extract_serialization_option(attrs: &[syn::Attribute]) -> Result<Option<String>, syn::Error> {
    for attr in attrs {
        if attr.path().is_ident("rpc_service") || attr.path().is_ident("rpc_client") {
            if let Meta::List(meta_list) = &attr.meta {
                let mut result: Option<String> = None;
                meta_list.parse_nested_meta(|meta| {
                    if meta.path.is_ident("serialize") {
                        let ser: LitStr = meta.value()?.parse()?;
                        if ser.value() == "bincode" {
                            result = Some("bincode".to_string());
                        } else if ser.value() == "serde" {
                            unimplemented!("serde serialization not implemented");
                            // result = Some("serde".to_string());
                        } else {
                            return Err(syn::Error::new(
                                meta_list.span(),
                                "'serialize' option expects one of: \"bincode\" | \"serde\" ",
                            ));
                        }
                    }
                    Ok(())
                })?;
                return Ok(result);
            }
        }
    }
    Ok(None)
}

enum VariantKind {
    ServiceRpc,
    ClientRpc,
    Response(Vec<String>),
}

fn extract_variant_attributes(attrs: &[syn::Attribute]) -> Result<VariantKind, syn::Error> {
    for attr in attrs {
        if attr.path().is_ident("response") {
            attr.meta.require_list()?;
            if let Meta::List(meta_list) = &attr.meta {
                let response_attrs = meta_list
                    .parse_args_with(Punctuated::<LitStr, Token![,]>::parse_terminated)
                    .map_err(|e| {
                        syn::Error::new(
                            meta_list.span(),
                            format!(
                                "{e}, 'response' expects a comma separated list of Variant names"
                            ),
                        )
                    })?;
                return Ok(VariantKind::Response(
                    response_attrs
                        .iter()
                        .map(|r| r.value())
                        .collect::<Vec<String>>(),
                ));
            }
        } else if attr.path().is_ident("client") {
            return Ok(VariantKind::ClientRpc);
        }
    }
    Ok(VariantKind::ServiceRpc)
}
