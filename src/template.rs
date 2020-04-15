use {handlebars::Handlebars, serde::Serialize, std::sync::Arc};

pub struct WithTemplate<T: Serialize> {
    pub name: &'static str,
    pub value: T,
}

impl<T: Serialize> WithTemplate<T> {
    pub fn render(self, hbs: Arc<Handlebars>) -> impl warp::Reply {
        warp::reply::html(
            hbs.render(self.name, &self.value)
                .unwrap_or_else(|err| format!("{}", err)),
        )
    }
}

pub fn init() -> Handlebars {
    let mut hb = Handlebars::new();
    hb.register_template_string("index", include_str!("./static/index.hbs"))
        .unwrap();
    hb.register_partial("nav", include_str!("./static/nav.hbs"))
        .unwrap();
    hb.register_partial("form", include_str!("./static/form.hbs"))
        .unwrap();
    hb.register_template_string("new", include_str!("./static/new.hbs"))
        .unwrap();
    hb.register_template_string("edit", include_str!("./static/edit.hbs"))
        .unwrap();
    hb
}
