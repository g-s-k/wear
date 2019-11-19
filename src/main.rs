use {
    chrono::{DateTime, Utc},
    handlebars::Handlebars,
    serde::{Deserialize, Serialize},
    serde_json::json,
    std::{
        collections::HashMap,
        error::Error,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc, Mutex,
        },
    },
    warp::{path, Filter},
};

mod utils;

struct WithTemplate<T: Serialize> {
    name: &'static str,
    value: T,
}

fn render<T>(template: WithTemplate<T>, hbs: Arc<Handlebars>) -> impl warp::Reply
where
    T: Serialize,
{
    hbs.render(template.name, &template.value)
        .unwrap_or_else(|err| err.description().to_owned())
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Item {
    name: String,
    description: String,
    #[serde(default)]
    count: usize,
    #[serde(default)]
    last: Option<DateTime<Utc>>,
    #[serde(default = "utils::default_color")]
    color: String,
}

type WrappedState = Arc<Mutex<HashMap<usize, Item>>>;

fn home_page(state: WrappedState) -> WithTemplate<serde_json::Value> {
    let state = state.lock().unwrap();
    let items = state
        .iter()
        .map(
            |(
                key,
                Item {
                    name,
                    description,
                    count,
                    last,
                    color,
                },
            )| {
                json!({
                    "key": key,
                    "name": name,
                    "description": description,
                    "count": count,
                    "hasLast": last.is_some(),
                    "last": last,
                    "lastFmt": last.map(|l| utils::format_since(l)),
                    "color": color,
                })
            },
        )
        .collect::<Vec<_>>();

    WithTemplate {
        name: "index",
        value: json!({"items": items, "numItems": items.len(), "user" : "warp"}),
    }
}

fn edit_page(
    idx: usize,
    Item {
        name,
        description,
        color,
        ..
    }: Item,
) -> WithTemplate<serde_json::Value> {
    WithTemplate {
        name: "edit",
        value: json!({
            "key": idx,
            "name": name,
            "description": description,
            "color": color,
        }),
    }
}

fn main() {
    let mut hb = Handlebars::new();
    hb.register_template_file("index", "./src/static/index.html")
        .unwrap();
    hb.register_template_file("edit", "./src/static/edit.html")
        .unwrap();
    let hb = Arc::new(hb);
    let hbars = move |with_template| render(with_template, hb.clone());

    let index = Arc::new(AtomicUsize::new(0));
    let with_index = warp::any().map(move || index.clone());
    let state = Arc::new(Mutex::new(HashMap::new()));
    let with_state = warp::any().map(move || state.clone());

    let index = warp::get2()
        .and(path::end())
        .and(with_state.clone())
        .map(home_page)
        .map(hbars.clone())
        .with(utils::html_header());

    let css = path("styles.css")
        .and(path::end())
        .and(warp::fs::file("./src/static/styles.css"));

    let post_item = warp::post2()
        .and(path::end())
        .and(with_state.clone())
        .and(with_index.clone())
        .and(warp::body::content_length_limit(1024 * 32))
        .and(warp::body::form())
        .and_then(
            |s: WrappedState, i: Arc<AtomicUsize>, item: Item| match s.lock() {
                Err(e) => Err(warp::reject::custom(e.description())),
                Ok(mut state) => {
                    state.insert(i.fetch_add(1, Ordering::Relaxed), item);
                    Ok(utils::go_home())
                }
            },
        );

    let edit_item = warp::get2()
        .and(path::param2())
        .and(path::end())
        .and(with_state.clone())
        .and_then(|i: usize, s: WrappedState| match s.lock() {
            Err(e) => Err(warp::reject::custom(e.description())),
            Ok(state) => {
                if let Some(v) = state.get(&i) {
                    Ok(edit_page(i, v.clone()))
                } else {
                    Err(warp::reject::not_found())
                }
            }
        })
        .map(hbars.clone())
        .with(utils::html_header());

    let update_item = warp::post2()
        .and(path::param2())
        .and(path::end())
        .and(with_state.clone())
        .and(warp::body::content_length_limit(1024 * 32))
        .and(warp::body::form())
        .and_then(
            |i: usize,
             s: WrappedState,
             Item {
                 name,
                 description,
                 color,
                 ..
             }: Item| {
                let mut state = match s.lock() {
                    Err(e) => return Err(warp::reject::custom(e.description())),
                    Ok(s) => s,
                };

                if let Some(v) = state.get_mut(&i) {
                    v.color = color;
                    v.name = name;
                    v.description = description;
                    Ok(utils::go_home())
                } else {
                    Err(warp::reject::not_found())
                }
            },
        );

    let increment_item = warp::post2()
        .and(path::param2())
        .and(warp::path("increment"))
        .and(path::end())
        .and(with_state.clone())
        .map(|i: usize, s: WrappedState| {
            if let Some(v) = s.lock().unwrap().get_mut(&i) {
                v.count += 1;
                v.last = Some(Utc::now());
            }

            utils::go_home()
        });

    let delete_item = warp::post2()
        .and(path::param2())
        .and(path("remove"))
        .and(path::end())
        .and(with_state.clone())
        .and_then(|i: usize, s: WrappedState| match s.lock() {
            Err(e) => Err(warp::reject::custom(e.description())),
            Ok(mut state) => {
                state.remove(&i);
                Ok(utils::go_home())
            }
        });

    let router = index.or(css).or(warp::path("item").and(
        post_item
            .or(edit_item)
            .or(update_item)
            .or(increment_item)
            .or(delete_item),
    ));
    warp::serve(router).run(([127, 0, 0, 1], 3000));
}
