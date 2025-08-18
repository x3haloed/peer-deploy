// Library entry for the hello component.
// Default build (workspace `cargo build`) does not compile the WASI HTTP component bindings.
// Enable the `component` feature and use `cargo component build` to build as a WASI HTTP component.

#![cfg_attr(feature = "component", no_main)]

#[cfg(feature = "component")]
mod bindings;

#[cfg(feature = "component")]
mod component_impl {
    #[allow(unused_imports)]
    use crate::bindings as bindings;

    use bindings::exports::wasi::http::incoming_handler::Guest;
    use bindings::wasi::http::types as http;

    struct Hello;

    impl Guest for Hello {
        fn handle(_req: http::IncomingRequest, out: http::ResponseOutparam) {
            let headers = http::Fields::new();
            let resp = http::OutgoingResponse::new(headers);
            let body = resp.body().expect("body");
            http::ResponseOutparam::set(out, Ok(resp));
            let mut w = body.write().expect("write");
            let _ = w.write(b"hello, world\n");
            drop(w);
            let _ = http::OutgoingBody::finish(body, None);
        }
    }

    bindings::export!(Hello with_types_in bindings);
}

#[cfg(not(feature = "component"))]
mod non_component_stub {
    // No-op library to keep workspace `cargo build` happy when not building the component target.
    pub const BUILD_INFO: &str = "hello lib (non-component build)";
}


