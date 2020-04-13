#![deny(clippy::all)]

use {
    chrono::{DateTime, Utc},
    handlebars::Handlebars,
    serde::{Deserialize, Serialize},
    serde_json::json,
    std::{
        collections::HashMap,
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
        .unwrap_or_else(|err| format!("{}", err))
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
    #[serde(
        deserialize_with = "utils::split_comma",
        serialize_with = "utils::join_comma"
    )]
    tags: Vec<String>,
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
                    tags,
                },
            )| {
                json!({
                    "key": key,
                    "name": name,
                    "description": description,
                    "count": count,
                    "hasLast": last.is_some(),
                    "last": last,
                    "lastFmt": last.map(utils::format_since),
                    "color": color,
                    "tags": tags.join(", "),
                })
            },
        )
        .collect::<Vec<_>>();

    WithTemplate {
        name: "index",
        value: json!({
            "items": items,
            "numItems": items.len(),
            "user" : "warp"
        }),
    }
}

fn edit_page(
    idx: usize,
    Item {
        name,
        description,
        color,
        tags,
        ..
    }: Item,
) -> WithTemplate<serde_json::Value> {
    WithTemplate {
        name: "edit",
        value: json!({
            "edit": true,
            "key": idx,
            "name": name,
            "description": description,
            "color": color,
            "tags": tags.join(", "),
        }),
    }
}

#[derive(Debug)]
struct MutexUnlockFailure;
impl warp::reject::Reject for MutexUnlockFailure {}

macro_rules! lock_mutex {
    ( $s:expr ) => {
        $s.lock()
            .map_err(|_| warp::reject::custom(MutexUnlockFailure))?
    };
}

async fn handle_post_item(
    s: WrappedState,
    i: Arc<AtomicUsize>,
    item: Item,
) -> Result<impl warp::Reply, warp::Rejection> {
    lock_mutex!(s).insert(i.fetch_add(1, Ordering::Relaxed), item);
    Ok(utils::go_home())
}

async fn handle_update_item(
    i: usize,
    s: WrappedState,
    Item {
        name,
        description,
        color,
        ..
    }: Item,
) -> Result<impl warp::Reply, warp::Rejection> {
    if let Some(v) = lock_mutex!(s).get_mut(&i) {
        v.color = color;
        v.name = name;
        v.description = description;
        Ok(utils::go_home())
    } else {
        Err(warp::reject::not_found())
    }
}

async fn handle_increment(i: usize, s: WrappedState) -> Result<impl warp::Reply, warp::Rejection> {
    if let Some(v) = lock_mutex!(s).get_mut(&i) {
        v.count += 1;
        v.last = Some(Utc::now());
        Ok(utils::go_home())
    } else {
        Err(warp::reject::not_found())
    }
}

async fn handle_delete_item(i: usize, s: WrappedState) -> Result<impl warp::Reply, warp::Rejection> {
    lock_mutex!(s).remove(&i);
    Ok(utils::go_home())
}

async fn handle_edit_form(
    i: usize,
    s: WrappedState,
) -> Result<WithTemplate<serde_json::Value>, warp::Rejection> {
    lock_mutex!(s)
        .get(&i)
        .map(|v| edit_page(i, v.clone()))
        .ok_or_else(warp::reject::not_found)
}

#[tokio::main]
async fn main() {
    let mut hb = Handlebars::new();
    hb.register_template_string("index", include_str!("./static/index.hbs"))
        .unwrap();
    hb.register_partial("entry", include_str!("./static/entry.hbs"))
        .unwrap();
    hb.register_partial("form", include_str!("./static/form.hbs"))
        .unwrap();
    hb.register_template_string("edit", include_str!("./static/edit.hbs"))
        .unwrap();
    let hb = Arc::new(hb);
    let hbars = move |with_template| render(with_template, hb.clone());

    let index = Arc::new(AtomicUsize::new(0));
    let with_index = warp::any().map(move || index.clone());
    let state = Arc::new(Mutex::new(HashMap::new()));
    let with_state = warp::any().map(move || state.clone());

    let index = warp::get()
        .and(path::end())
        .and(with_state.clone())
        .map(home_page)
        .map(hbars.clone())
        .with(utils::html_header());

    let css = path("styles.css")
        .and(path::end())
        .map(|| include_str!("./static/styles.css"))
        .with(utils::css_header());

    let post_item = warp::post()
        .and(path::end())
        .and(with_state.clone())
        .and(with_index)
        .and(warp::body::content_length_limit(1024 * 32))
        .and(warp::body::form())
        .and_then(handle_post_item);

    let edit_item = warp::get()
        .and(path::param())
        .and(path::end())
        .and(with_state.clone())
        .and_then(handle_edit_form)
        .map(hbars)
        .with(utils::html_header());

    let update_item = warp::post()
        .and(path::param())
        .and(path::end())
        .and(with_state.clone())
        .and(warp::body::content_length_limit(1024 * 32))
        .and(warp::body::form())
        .and_then(handle_update_item);

    let increment_item = warp::post()
        .and(path::param())
        .and(warp::path("increment"))
        .and(path::end())
        .and(with_state.clone())
        .and_then(handle_increment);

    let delete_item = warp::post()
        .and(path::param())
        .and(path("remove"))
        .and(path::end())
        .and(with_state)
        .and_then(handle_delete_item);

    let router = index.or(css).or(warp::path("item").and(
        post_item
            .or(edit_item)
            .or(update_item)
            .or(increment_item)
            .or(delete_item),
    ));

    warp::serve(router).run(([0, 0, 0, 0], 3000)).await;
}
