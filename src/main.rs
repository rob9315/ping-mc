mod bedrock;
mod cors;
mod java;

#[allow(unused)]
use std::sync::atomic::AtomicUsize;

use cors::Cors;
use trust_dns_resolver::TokioAsyncResolver;

#[macro_use]
extern crate rocket;

#[launch]
fn rocket() -> _ {
    rocket::build()
        .manage(dns())
        .attach(Cors)
        .mount("/", routes![java::java, bedrock::bedrock])
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
