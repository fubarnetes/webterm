extern crate actix;
extern crate actix_web;
extern crate webterm;

use actix_web::{fs::StaticFiles, server, App};
use webterm::WebTermExt;

use std::process::Command;

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
            .webterm_socket("/websocket", |_req| {
                let mut cmd = Command::new("/bin/sh");
                cmd.env("TERM", "xterm");
                cmd
            })
            .webterm_ui("/", "/websocket", "/static")
    })
    .bind("127.0.0.1:8080")
    .unwrap()
    .run();
}
