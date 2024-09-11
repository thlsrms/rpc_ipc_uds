use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};

use proc_macro::TokenStream;
use quote::quote;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{parse_macro_input, DeriveInput, Fields, LitStr, Meta, Token};

#[proc_macro_derive(RpcService, attributes(rpc_service, response))]
pub fn rpc_service_derive(input: TokenStream) -> TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);
    service_impl(ast)
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

fn service_impl(ast: DeriveInput) -> Result<proc_macro2::TokenStream, syn::Error> {
    let name = ast.ident;
    let service_trait_name = quote::format_ident!("{}Service", name);

    let variants = match &ast.data {
        syn::Data::Enum(data_enum) => &data_enum.variants,
        _ => panic!("#[derive(RpcService)] is only valid for enums"),
    };

    let response_enum_name = quote::format_ident!("{}ResponsePayload", name);
    let error_name = quote::format_ident!("{}Error", name);

    let serialization_option = extract_serialization_option(&ast.attrs)?;
    let rpc_structs = generate_rpc_request_response(&name, serialization_option.as_deref());
    let (request_variants, payload_enums) =
        generate_payload_enums(&name, variants, serialization_option.as_deref())?;

    let mut service_methods = vec![];

    for variant in &request_variants {
        let variant_name = &variant.ident;

        let method_name = to_snake_case(&variant_name.to_string());

        match &variant.fields {
            Fields::Unnamed(fields) => {
                let args = fields.unnamed.iter().enumerate().map(|(i, f)| {
                    let ident = quote::format_ident!("arg{}", i);
                    let ty = &f.ty;
                    quote! { #ident: #ty }
                });

                service_methods.push(quote! {
                    fn #method_name(&self, #(#args),*) -> Result<#response_enum_name, #error_name>;
                });
            }
            Fields::Named(fields) => {
                let args = fields.named.iter().map(|f| {
                    let ident = f.ident.as_ref().unwrap();
                    let ty = &f.ty;
                    quote! { #ident: #ty }
                });

                service_methods.push(quote! {
                    fn #method_name(&self, #(#args),*) -> Result<#response_enum_name, #error_name>;
                });
            }
            Fields::Unit => {
                service_methods.push(quote! {
                    fn #method_name(&self) -> Result<#response_enum_name, #error_name>;
                });
            }
        }
    }

    let service_impl = quote! {
        #rpc_structs

        #payload_enums

        pub trait #service_trait_name {
            #(#service_methods)*
        }
    };

    Ok(service_impl)
}

#[proc_macro_derive(RpcClient, attributes(rpc_client, response))]
pub fn rpc_client_derive(input: TokenStream) -> TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);

    client_impl(ast)
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

fn client_impl(ast: DeriveInput) -> Result<proc_macro2::TokenStream, syn::Error> {
    let name = ast.ident;
    let client_trait_name = quote::format_ident!("{}Client", name);

    let variants = match &ast.data {
        syn::Data::Enum(data_enum) => &data_enum.variants,
        _ => panic!("#[derive(RpcClient)] is only valid for enums"),
    };

    let request_enum_name = quote::format_ident!("{}RequestPayload", name);
    let request_type = quote::format_ident!("{}Request", name);

    let serialization_option = extract_serialization_option(&ast.attrs)?;
    let rpc_structs = generate_rpc_request_response(&name, serialization_option.as_deref());
    let (request_variants, payload_enums) =
        generate_payload_enums(&name, variants, serialization_option.as_deref())?;

    let mut client_methods = vec![];

    for variant in request_variants {
        let variant_name = &variant.ident;
        let method_name = to_snake_case(&variant_name.to_string());

        match &variant.fields {
            Fields::Unnamed(fields) => {
                let args = fields.unnamed.iter().enumerate().map(|(i, f)| {
                    let ident = quote::format_ident!("arg{}", i);
                    let ty = &f.ty;
                    (quote! { #ident: #ty }, quote! { #ident })
                });

                let (args, args_for_call): (Vec<_>, Vec<_>) = args.unzip();

                client_methods.push(quote! {
                    fn #method_name(&self, #(#args),*) -> Result<(), Box<dyn std::error::Error>> {
                        self.send_request(#request_type {
                            id: self.generate_request_id(),
                            payload: #request_enum_name::#variant_name( #(#args_for_call),* ),
                        })
                    }
                });
            }
            Fields::Named(fields) => {
                let args = fields.named.iter().map(|f| {
                    let ident = f.ident.as_ref().unwrap();
                    let ty = &f.ty;
                    (quote! { #ident: #ty }, quote! { #ident })
                });

                let (args, args_for_call): (Vec<_>, Vec<_>) = args.unzip();

                client_methods.push(quote! {
                    fn #method_name(&self, #(#args),*) -> Result<(), Box<dyn std::error::Error>> {
                        self.send_request(#request_type {
                            id: self.generate_request_id(),
                            payload: #request_enum_name::#variant_name { #(#args_for_call),* },
                        } )
                    }
                });
            }
            Fields::Unit => {
                client_methods.push(quote! {
                    fn #method_name(&self) -> Result<(), Box<dyn std::error::Error>> {
                        self.send_request(#request_type {
                            id: self.generate_request_id(),
                            payload: #request_enum_name::#variant_name,
                        })
                    }
                });
            }
        }
    }

    let client_impl = quote! {
        #rpc_structs

        #payload_enums

        pub trait #client_trait_name {
            fn send_request(&self, request: #request_type) -> Result<(), Box<dyn std::error::Error>>;
            fn poll_responses(&self);
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

fn generate_payload_enums<'a>(
    name: &syn::Ident,
    variants: &'a Punctuated<syn::Variant, Token![,]>,
    serialization: Option<&str>,
) -> Result<(Vec<&'a syn::Variant>, proc_macro2::TokenStream), syn::Error> {
    let request_enum_name = quote::format_ident!("{}RequestPayload", name);
    let response_enum_name = quote::format_ident!("{}ResponsePayload", name);

    let mut responses_map: HashMap<String, String> = HashMap::new();
    let mut response_variants: HashMap<String, &syn::Variant> = HashMap::new();
    let mut request_variants: Vec<&syn::Variant> = vec![];

    for variant in variants {
        let variant_name = &variant.ident;
        let variant_attrs = &variant.attrs;
        // Treat this variant as response if marked with the [response(variants)] attribute
        // Use the variant names to identify which variant uses this one as response
        let variant_is_response_to = extract_variant_response_otions(variant_attrs)?;
        if variant_is_response_to.is_empty() {
            request_variants.push(variant);
        } else {
            response_variants.insert(variant_name.to_string(), variant);
            for request_variant in variant_is_response_to.into_iter() {
                responses_map.insert(request_variant, variant_name.to_string());
            }
        }
    }

    let mut res_variants = vec![];
    for variant in &request_variants {
        let variant_name = &variant.ident;
        let response_variant = if let Some(res) = responses_map.get(&variant_name.to_string()) {
            response_variants.get(res).unwrap()
        } else {
            variant
        };

        match &response_variant.fields {
            Fields::Unnamed(fields) => {
                let args = fields.unnamed.iter().map(|f| {
                    let ty = &f.ty;
                    quote! { #ty }
                });

                res_variants.push(quote! { #variant_name(#(#args ),*)});
            }
            Fields::Named(fields) => {
                let args = fields.named.iter().map(|f| {
                    let ident = f.ident.as_ref().unwrap();
                    let ty = &f.ty;
                    quote! { #ident: #ty }
                });

                res_variants.push(quote! { #variant_name { #(#args ),* }});
            }
            Fields::Unit => {
                res_variants.push(quote! { #variant_name });
            }
        }
    }

    // Check if we've already generated this
    let mut generated = GENERATED_TYPES.lock().unwrap();
    if generated.contains(&request_enum_name.to_string()) {
        return Ok((request_variants, proc_macro2::TokenStream::new()));
    }

    let serialization_derives = match serialization {
        Some("bincode") => quote! {
            #[derive(::bincode::Encode, ::bincode::Decode, Debug, Clone)]
        },
        Some("serde") => quote! {
            #[derive(::serde::Serialize, ::serde::Deserialize, Debug, Clone)]
        },
        _ => quote! {
            #[derive(Debug, Clone)]
        },
    };

    let request_enum = quote! {
        pub enum #request_enum_name {
            #(#request_variants),*
        }
    };

    let response_enum = quote! {
        pub enum #response_enum_name {
            #(#res_variants),*
        }
    };

    generated.insert(request_enum_name.to_string());
    generated.insert(response_enum_name.to_string());
    Ok((
        request_variants,
        quote! {
            #serialization_derives
            #request_enum

            #serialization_derives
            #response_enum
        },
    ))
}

fn generate_rpc_request_response(
    name: &syn::Ident,
    serialization: Option<&str>,
) -> proc_macro2::TokenStream {
    let request_name = quote::format_ident!("{}Request", name);
    let response_name = quote::format_ident!("{}Response", name);
    let request_enum_name = quote::format_ident!("{}RequestPayload", name);
    let response_enum_name = quote::format_ident!("{}ResponsePayload", name);
    let error_kind_name = quote::format_ident!("{}ErrorKind", name);
    let error_name = quote::format_ident!("{}Error", name);

    // Check if we've already generated this
    let mut generated = GENERATED_TYPES.lock().unwrap();
    if generated.contains(&request_name.to_string()) {
        return proc_macro2::TokenStream::new();
    }

    let serialization_derives = match serialization {
        Some("bincode") => quote! {
            #[derive(::bincode::Encode, ::bincode::Decode, Debug, Clone)]
        },
        Some("serde") => quote! {
            #[derive(::serde::Serialize, ::serde::Deserialize, Debug, Clone)]
        },
        _ => quote! {
            #[derive(Debug, Clone)]
        },
    };

    generated.insert(request_name.to_string());
    quote! {
        #serialization_derives
        pub struct #request_name {
            pub id: u32,
            pub payload: #request_enum_name,
        }

        #serialization_derives
        pub struct #response_name {
            pub id: u32,
            pub payload: Result<#response_enum_name, #error_name>,
        }

        #serialization_derives
        pub enum #error_kind_name {
            Timeout,
            InvalidArgument,
            PermissionDenied,
            NotFound,
            Unimplemented,
            Unreachable,
            Internal,
        }

        impl std::fmt::Display for #error_kind_name {
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
        pub struct #error_name {
            error: #error_kind_name,
            description: String,
        }

        impl #error_name {
            pub fn new(error: #error_kind_name) -> Self {
                Self {
                    description: error.to_string(),
                    error,
                }
            }

            fn set_description(&mut self, description: impl Into<String>) {
                self.description = description.into();
            }
        }

        impl std::fmt::Display for #error_name {
            fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(f, "{}", self.error)
            }
        }

        impl std::error::Error for #error_name {
            fn description(&self) -> &str {
                &self.description
            }
        }
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

fn extract_variant_response_otions(attrs: &[syn::Attribute]) -> Result<Vec<String>, syn::Error> {
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
                return Ok(response_attrs
                    .iter()
                    .map(|r| r.value())
                    .collect::<Vec<String>>());
            }
        }
    }
    Ok(vec![])
}
