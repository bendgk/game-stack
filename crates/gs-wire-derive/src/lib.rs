//! `#[derive(Wire)]` — generates `encode`, `decode` and `schema` for structs and enums.
//!
//! Field attributes:
//! - `#[wire(skip)]` — not sent; decoded as `Default::default()` (render-only / derived data).
//!
//! The generated `schema` writes field and variant names in declaration order, so renaming,
//! reordering, adding or retyping anything changes the protocol fingerprint. That is the
//! point: a client built from different source than the server is refused at `Hello`.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as Ts;
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Fields, GenericParam, parse_macro_input, parse_quote};

#[proc_macro_derive(Wire, attributes(wire))]
pub fn derive_wire(input: TokenStream) -> TokenStream {
    let mut input = parse_macro_input!(input as DeriveInput);
    match expand(&mut input) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn is_skip(attrs: &[syn::Attribute]) -> syn::Result<bool> {
    let mut skip = false;
    for attr in attrs {
        if !attr.path().is_ident("wire") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("skip") {
                skip = true;
                Ok(())
            } else {
                Err(meta.error("unknown wire attribute (supported: skip)"))
            }
        })?;
    }
    Ok(skip)
}

fn expand(input: &mut DeriveInput) -> syn::Result<Ts> {
    for p in input.generics.params.iter_mut() {
        if let GenericParam::Type(t) = p {
            t.bounds.push(parse_quote!(::gs_wire::Wire));
        }
    }
    let name = &input.ident;
    let (impl_g, ty_g, where_g) = input.generics.split_for_impl();

    let (enc, dec, schema) = match &input.data {
        Data::Struct(s) => {
            let (enc, dec, schema) = fields_code(&s.fields, quote!(Self), true)?;
            let schema = quote! { s.record(|s| { #schema }); };
            (enc, dec, schema)
        }
        Data::Enum(e) => {
            if e.variants.is_empty() {
                return Err(syn::Error::new_spanned(name, "Wire cannot be derived for an empty enum"));
            }
            let mut enc_arms = Vec::new();
            let mut dec_arms = Vec::new();
            let mut schema_arms = Vec::new();
            for (idx, v) in e.variants.iter().enumerate() {
                let idx = idx as u32;
                let vname = &v.ident;
                let vstr = vname.to_string();
                let path = quote!(Self::#vname);
                let (venc, vdec, vschema) = fields_code(&v.fields, path.clone(), false)?;
                let pat = match &v.fields {
                    Fields::Named(f) => {
                        let ids = f.named.iter().map(|f| f.ident.clone().unwrap());
                        quote!(#path { #(#ids),* })
                    }
                    Fields::Unnamed(f) => {
                        let ids = (0..f.unnamed.len()).map(|i| format_ident!("f{}", i));
                        quote!(#path ( #(#ids),* ))
                    }
                    Fields::Unit => quote!(#path),
                };
                enc_arms.push(quote! { #pat => { w.varint(#idx as u64); #venc } });
                dec_arms.push(quote! { #idx => { #vdec } });
                schema_arms.push(quote! { s.variant(#vstr, |s| { #vschema }); });
            }
            let n = e.variants.len() as u32;
            let enc = quote! {
                #[allow(unused_variables)]
                match self { #(#enc_arms)* }
            };
            let dec = quote! {
                let tag = r.varint()?;
                let tag: u32 = if tag < #n as u64 { tag as u32 } else { return Err(::gs_wire::WireError::BadTag(tag)) };
                match tag { #(#dec_arms)* _ => unreachable!() }
            };
            let schema = quote! { s.choice(|s| { #(#schema_arms)* }); };
            (enc, dec, schema)
        }
        Data::Union(u) => {
            return Err(syn::Error::new_spanned(u.union_token, "Wire cannot be derived for unions"));
        }
    };

    Ok(quote! {
        impl #impl_g ::gs_wire::Wire for #name #ty_g #where_g {
            #[inline]
            fn encode(&self, w: &mut ::gs_wire::Writer) { #enc }
            #[inline]
            fn decode(r: &mut ::gs_wire::Reader<'_>) -> ::core::result::Result<Self, ::gs_wire::WireError> {
                r.enter()?;
                let out = { #dec };
                r.leave();
                Ok(out)
            }
            fn schema(s: &mut ::gs_wire::Schema) {
                s.named(::gs_wire::short_type_name::<Self>(), |s| { #schema });
            }
        }
    })
}

/// Returns (encode, decode-expression, schema) for one field list.
/// `is_struct`: encode reads `self.field`; otherwise the match arm bound the fields by name.
fn fields_code(fields: &Fields, ctor: Ts, is_struct: bool) -> syn::Result<(Ts, Ts, Ts)> {
    let mut enc = Vec::new();
    let mut dec = Vec::new();
    let mut schema = Vec::new();
    match fields {
        Fields::Named(f) => {
            for field in &f.named {
                let id = field.ident.clone().unwrap();
                let ty = &field.ty;
                let fname = id.to_string();
                if is_skip(&field.attrs)? {
                    dec.push(quote!(#id: ::core::default::Default::default()));
                    continue;
                }
                let access = if is_struct { quote!(&self.#id) } else { quote!(#id) };
                enc.push(quote!(::gs_wire::Wire::encode(#access, w);));
                dec.push(quote!(#id: <#ty as ::gs_wire::Wire>::decode(r)?));
                schema.push(quote!(s.field(#fname, |s| <#ty as ::gs_wire::Wire>::schema(s));));
            }
            Ok((quote!(#(#enc)*), quote!(#ctor { #(#dec),* }), quote!(#(#schema)*)))
        }
        Fields::Unnamed(f) => {
            for (i, field) in f.unnamed.iter().enumerate() {
                let ty = &field.ty;
                let fname = i.to_string();
                if is_skip(&field.attrs)? {
                    dec.push(quote!(::core::default::Default::default()));
                    continue;
                }
                let access = if is_struct {
                    let idx = syn::Index::from(i);
                    quote!(&self.#idx)
                } else {
                    let id = format_ident!("f{}", i);
                    quote!(#id)
                };
                enc.push(quote!(::gs_wire::Wire::encode(#access, w);));
                dec.push(quote!(<#ty as ::gs_wire::Wire>::decode(r)?));
                schema.push(quote!(s.field(#fname, |s| <#ty as ::gs_wire::Wire>::schema(s));));
            }
            Ok((quote!(#(#enc)*), quote!(#ctor ( #(#dec),* )), quote!(#(#schema)*)))
        }
        Fields::Unit => Ok((quote!(), quote!(#ctor), quote!())),
    }
}
