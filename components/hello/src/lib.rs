// Library entry for the hello component.
// Default build (workspace `cargo build`) does not compile the WASI HTTP component bindings.
// Enable the `component` feature and use `cargo component build` to build as a WASI HTTP component.

#[cfg(feature = "component")]
mod component_impl {
    #![no_main]
    wit_bindgen::generate!({ world: "hello", path: "wit" });

    use exports::wasi::http::incoming_handler::Guest;
    use wasi::http::types as http;

    struct Hello;

    impl Guest for Hello {
        fn handle(_req: http::IncomingRequest, out: http::ResponseOutparam) {
            let headers = http::Fields::new();
            let resp = http::OutgoingResponse::new(headers);
            let body = resp.body().expect("body");
            out.set(Ok(resp));
            let mut w = body.write().expect("write");
            use std::io::Write;
            let _ = w.write_all(b"hello, world\n");
            drop(w);
            let _ = http::OutgoingBody::finish(body, None);
        }
    }

    #[export_name = "_start"]
    pub extern "C" fn _start() {}
}

#[cfg(not(feature = "component"))]
mod non_component_stub {
    // No-op library to keep workspace `cargo build` happy when not building the component target.
    pub const BUILD_INFO: &str = "hello lib (non-component build)";
}


