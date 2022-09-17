#![allow(clippy::let_unit_value)]

mod bedrock;
mod cors;
mod java;

use std::path::PathBuf;
#[allow(unused)]
use std::sync::atomic::AtomicUsize;

use cors::Cors;
use trust_dns_resolver::TokioAsyncResolver;

#[macro_use]
extern crate rocket;

#[options("/<_path..>")]
pub fn options(_path: PathBuf) {}

#[launch]
fn rocket() -> _ {
    rocket::build()
        .manage(dns())
        .attach(Cors)
        .mount("/", routes![java::java, bedrock::bedrock, options,])
}

fn dns() -> TokioAsyncResolver {
    use trust_dns_resolver::{
        config::{ResolverConfig, ResolverOpts},
        TokioHandle,
    };
    TokioAsyncResolver::tokio_from_system_conf().unwrap_or_else(|_| {
        TokioAsyncResolver::new(
            ResolverConfig::default(),
            ResolverOpts::default(),
            TokioHandle,
        )
        .expect("couldn't construct dns resolver")
    })
}
