#[macro_use]
extern crate lazy_static;

use actix_web::{App, HttpServer};
use structopt::StructOpt;
use webterm::WebTermExt;

use std::process::Command;

#[derive(StructOpt, Debug)]
#[structopt(name = "webterm-server")]
struct Opt {
    /// The port to listen on
    #[structopt(short, long, default_value = "8080")]
    port: u16,

    /// The host or IP to listen on
    #[structopt(short, long, default_value = "localhost")]
    host: String,

    /// The command to execute
    #[structopt(short, long, default_value = "/bin/sh")]
    command: String,
}

lazy_static! {
    static ref OPT: Opt = Opt::from_args();
}

fn main() {
    pretty_env_logger::init();

    HttpServer::new(|| {
        App::new()
            .service(actix_files::Files::new("/static", "./node_modules"))
            .webterm_socket("/websocket", |_req| {
                let mut cmd = Command::new(OPT.command.clone());
                cmd.env("TERM", "xterm");
                cmd
            })
            .webterm_ui("/", "/websocket", "/static")
    })
    .bind(format!("{}:{}", OPT.host, OPT.port))
    .unwrap()
    .run()
    .unwrap();
}
