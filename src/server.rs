extern crate actix;
extern crate actix_web;
extern crate webterm;

use actix_web::{fs::NamedFile, fs::StaticFiles, server, App, HttpRequest, Result};
use webterm::WebTermExt;

fn index(_req: &HttpRequest) -> Result<NamedFile> {
    Ok(NamedFile::open("static/term.html")?)
}

fn main() {
    pretty_env_logger::init();

    server::new(|| {
        App::new()
            .handler(
                "/static",
                StaticFiles::new("node_modules")
                    .unwrap()
                    .show_files_listing(),
            )
            .webterm_socket("/websocket")
            .resource("/", |r| r.f(index))
    })
    .bind("127.0.0.1:8080")
    .unwrap()
    .run();
}
