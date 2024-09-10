use std::sync::Once;

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput, Fields, LitStr, Meta};

#[proc_macro_derive(RpcService, attributes(rpc_service))]
pub fn rpc_service_derive(input: TokenStream) -> TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);

    let name = ast.ident;
    let service_trait_name = quote::format_ident!("{}Service", name);

    let variants = match &ast.data {
        syn::Data::Enum(data_enum) => &data_enum.variants,
        _ => panic!("#[derive(RpcService)] is only valid for enums"),
    };

    let serialization_option = extract_serialization_option(&ast.attrs);
    let mut rpc_structs = proc_macro2::TokenStream::new();

    INIT.call_once(|| {
        // Generate the Request / Response structs only once for compilation
        rpc_structs = generate_rpc_request_response(&name, serialization_option.as_deref());
    });
    let response_type = quote::format_ident!("{}Response", name);

    let mut service_methods = vec![];

    for variant in variants {
        let variant_name = &variant.ident;
        let method_name = to_snake_case(&variant_name.to_string());

        match &variant.fields {
            Fields::Unnamed(fields) => {
                let args_with_types = fields.unnamed.iter().enumerate().map(|(i, f)| {
                    let ident = quote::format_ident!("arg{}", i);
                    let ty = &f.ty;
                    quote! { #ident: #ty }
                });

                service_methods.push(quote! {
                    fn #method_name(&self, #(#args_with_types),*) -> #response_type;
                });
            }
            Fields::Named(fields) => {
                let args_with_types = fields.named.iter().map(|f| {
                    let ident = f.ident.as_ref().unwrap();
                    let ty = &f.ty;
                    quote! { #ident: #ty }
                });

                service_methods.push(quote! {
                    fn #method_name(&self, #(#args_with_types),*) -> #response_type;
                });
            }
            Fields::Unit => {
                service_methods.push(quote! {
                    fn #method_name(&self) -> #response_type;
                });
            }
        }
    }

    let service_impl = quote! {
        #rpc_structs

        pub trait #service_trait_name {
            #(#service_methods)*
        }
    };

    TokenStream::from(service_impl)
}

#[proc_macro_derive(RpcClient, attributes(rpc_client))]
pub fn rpc_client_derive(input: TokenStream) -> TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);

    let name = ast.ident;
    let client_trait_name = quote::format_ident!("{}Client", name);

    let variants = match &ast.data {
        syn::Data::Enum(data_enum) => &data_enum.variants,
        _ => panic!("#[derive(RpcService)] is only valid for enums"),
    };

    let serialization_option = extract_serialization_option(&ast.attrs);
    let mut rpc_structs = proc_macro2::TokenStream::new();

    INIT.call_once(|| {
        // Generate the Request / Response structs only once for compilation
        rpc_structs = generate_rpc_request_response(&name, serialization_option.as_deref());
    });

    let mut client_methods = vec![];

    for variant in variants {
        let variant_name = &variant.ident;
        let method_name = to_snake_case(&variant_name.to_string());

        match &variant.fields {
            Fields::Unnamed(fields) => {
                let args = fields.unnamed.iter().enumerate().map(|(i, f)| {
                    let ident = quote::format_ident!("arg{}", i);
                    let ty = &f.ty;
                    quote! { #ident: #ty }
                });

                let args_for_call = fields.unnamed.iter().enumerate().map(|(i, _)| {
                    let ident = quote::format_ident!("arg{}", i);
                    quote! { #ident }
                });

                client_methods.push(quote! {
                    fn #method_name(&self, #(#args),*) -> Result<(), Box<dyn std::error::Error>> {
                        self.send_request(#name::#variant_name( #(#args_for_call),* ))
                    }
                });
            }
            Fields::Named(fields) => {
                let args = fields.named.iter().map(|f| {
                    let ident = f.ident.as_ref().unwrap();
                    let ty = &f.ty;
                    quote! { #ident: #ty }
                });

                let args_for_call = fields.named.iter().map(|f| {
                    let ident = &f.ident;
                    quote! { #ident }
                });

                client_methods.push(quote! {
                    fn #method_name(&self, #(#args),*) -> Result<(), Box<dyn std::error::Error>> {
                        self.send_request(#name::#variant_name { #(#args_for_call),* } )
                    }
                });
            }
            Fields::Unit => {
                client_methods.push(quote! {
                    fn #method_name(&self) -> Result<(), Box<dyn std::error::Error>> {
                        self.send_request(#name::#variant_name)
                    }
                });
            }
        }
    }

    let client_impl = quote! {
        #rpc_structs

        pub trait #client_trait_name {
            fn send_request(&self, request: #name) -> Result<(), Box<dyn std::error::Error>>;
            fn poll_responses(&self);

            #(#client_methods)*
        }
    };

    TokenStream::from(client_impl)
}

fn to_snake_case(s: &str) -> proc_macro2::Ident {
    println!("to_snake_case {s}");
    let mut snake = String::new();
    for (i, ch) in s.chars().enumerate() {
        if ch.is_uppercase() && i != 0 {
            snake.push('_');
        }
        snake.push(ch.to_ascii_lowercase());
    }
    println!("to_snake_case final {snake}");
    proc_macro2::Ident::new(&snake, proc_macro2::Span::call_site())
}

// Static flag to ensure RpcRequest and RpcResponse are only generated once.
static INIT: Once = Once::new();

fn generate_rpc_request_response(
    name: &syn::Ident,
    serialization: Option<&str>,
) -> proc_macro2::TokenStream {
    let serialization_derives = match serialization {
        Some("bincode") => quote! {
            #[derive(::bincode::Encode, ::bincode::Decode)]
        },
        Some("serde") => quote! {
            #[derive(::serde::Serialize, ::serde::Deserialize)]
        },
        _ => quote! {},
    };

    let request_name = quote::format_ident!("{}Request", name);
    let response_name = quote::format_ident!("{}Response", name);
    quote! {
        #serialization_derives
        pub struct #request_name {
            pub id: u32,
            pub data: #name,
        }

        #serialization_derives
        pub struct #response_name {
            pub id: u32,
            pub data: Option<#name>,
            pub error: Option<String>,
        }
    }
}

fn extract_serialization_option(attrs: &[syn::Attribute]) -> Option<String> {
    for attr in attrs {
        if let Meta::List(meta_list) = &attr.meta {
            if meta_list.path.is_ident("rpc_service") || meta_list.path.is_ident("rpc_client") {
                let mut result: Option<String> = None;
                meta_list
                    .parse_nested_meta(|nested| {
                        if nested.path.is_ident("serialize") {
                            let ser: LitStr = nested.value()?.parse()?;
                            if ser.value() == "bincode" {
                                result = Some("bincode".to_string());
                            } else if ser.value() == "serde" {
                                result = Some("serde".to_string());
                            }
                        }
                        Ok(())
                    })
                    .ok()?;
                return result;
            }
        }
    }
    None
}
