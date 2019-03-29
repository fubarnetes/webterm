use askama::Template;

#[derive(Template)]
#[template(path = "term.html")]
pub struct WebTerm {
    websocket_path: String,
    static_path: String,
}

impl WebTerm {
    pub fn new<S>(websocket_path: S, static_path: S) -> Self
    where
        S: Into<String>,
    {
        Self {
            websocket_path: websocket_path.into(),
            static_path: static_path.into(),
        }
    }
}
