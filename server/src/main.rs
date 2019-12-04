extern crate hyper;

use hyper::{service, Request, Response, Body, Server, StatusCode};
use hyper::header::{HeaderMap, HeaderName, HeaderValue};
use futures::{future::{self, Either}, Future, Stream};
use serde_json::json;
use lazy_static::lazy_static;
use ed25519_dalek::{PublicKey, Signature};
use std::{env, fs};
use hpos_state_core::state::State;
use base64::decode_config;

lazy_static! {
    static ref X_HPOS_ADMIN_SIGNATURE: HeaderName = HeaderName::from_lowercase(b"x-hpos-admin-signature").unwrap();
    static ref X_ORIGINAL_URI: HeaderName = HeaderName::from_lowercase(b"x-original-uri").unwrap();
    static ref HP_PUBLIC_KEY: PublicKey = read_hp_pubkey();
}

// Create response based on the request parameters
fn create_response(req: Request<Body>) -> impl Future<Item = Response<Body>, Error = hyper::Error> {
    let (parts, body) = req.into_parts();

    match parts.uri.path() {
        "/" => {
            let entire_body = body.concat2();
            let res = entire_body.map( |body| {
                // Extract X-Original-URI header value, panic for no header
                let req_uri = match parts.headers.get(&*X_ORIGINAL_URI) {
                    Some(s) => s.to_str().unwrap(),
                    None => panic!("Request does not contain \"X-Original-URI\" header."),
                };
                let body_string = String::from_utf8(body.to_vec()).expect("Found invalid UTF-8");
                let payload = create_payload(parts.method.to_string(), req_uri.to_string(), body_string);
                let is_verified = verify_request(payload, parts.headers);
                respond_success(is_verified)
            });

            Either::A(res)
        }
        _ => {
            let res = future::ok(respond_success(false));
            Either::B(res)
        }
    }
}

fn create_payload (method: String, uri: String, body_string: String) -> String {
    let d = json!({
        "method": method.to_lowercase(), // make sure verb is to lowercase
        "uri": uri,
        "body": body_string
    }); 

    // Serialize it to a JSON string.
    serde_json::to_string(&d).unwrap()
}

fn verify_request(payload: String, headers: HeaderMap<HeaderValue>) -> bool {
    // Retrieve X-Hpos-Admin-Signature, direct to 401 on error
    let signature_base64 = match headers.get(&*X_HPOS_ADMIN_SIGNATURE) {
        Some(s) => s.to_str().unwrap(),
        None => return false,
    };

    // Base64 decode signature, direct to 401 on error
    let signature_vec = match decode_config(&signature_base64, base64::STANDARD_NO_PAD) {
        Ok(s) => s,
        _ => return false,
    };

    // Convert signature to Signature type, direct to 401 on error
    let signature_bytes = match Signature::from_bytes(&signature_vec) {
        Ok(s) => s,
        _ => return false,
    };

    let public_key = &*HP_PUBLIC_KEY;
    // verify payload, direct to 401 on error
    match public_key.verify(&payload.as_bytes(), &signature_bytes) {
        Ok(_) => return true,
        _ => return false
    }
}

fn respond_success (is_verified: bool) -> hyper::Response<Body> {
    // construct response based on verification status
    match is_verified {
        true => {
            Response::builder()
                .status(StatusCode::OK)
                .body(Body::empty())
                .unwrap()
        },
        _ => {
            Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .body(Body::empty())
                .unwrap()
        }
    }
}

fn read_hp_pubkey() -> PublicKey {
    let hpos_state_path = env::var("HPOS_STATE_PATH").expect("HPOS_STATE_PATH environmental variable is not present");

    println!("Reading HP Admin Public Key from {}.", hpos_state_path);

    // Read from path
    let contents = fs::read(hpos_state_path)
        .expect("Something went wrong reading HP Admin Public Key from file");

    // Parse content
    let hpos_state: State = serde_json::from_slice(&contents).unwrap();
    hpos_state.get_admin_public_key().expect("HP Admin Public key seems to be corrupted")
}

fn main() {
    // Listen on http socket port 2884 - "auth" in phonespell
    let listen_address = ([127,0,0,1], 2884).into();

    // Trigger lazy static to see if HP_PUBLIC_KEY assignment creates panic
    let _ = &*HP_PUBLIC_KEY;

    // Create a `Service` from servicing function
    let new_svc = || {
        service::service_fn(create_response)
    };

    let server = Server::bind(&listen_address)
        .serve(new_svc)
        .map_err(|e| {
            eprintln!("server error: {}", e);
        });

    println!("Listening on http://{}", listen_address);

    // Run forever
    hyper::rt::run(server);
}
