#![deny(clippy::all)]

use {
    anyhow::Context,
    chrono::{DateTime, Utc},
    chrono_humanize::Humanize,
    handlebars::Handlebars,
    serde::{Deserialize, Serialize},
    serde_json::json,
    std::sync::Arc,
    tokio::{signal, sync::oneshot},
    warp::{path, Filter},
};

mod db;
mod template;
mod utils;

use {db::Connection, template::WithTemplate};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let hb = template::init().context("Failed to initialize templating engine")?;
    let conn = Connection::new()
        .await
        .context("Failed to connect to database")?;

    // set up the server in a way that lets us shut it down from the outside
    let (tx, rx) = oneshot::channel();
    let (_address, server) = warp::serve(new_router(hb, conn.clone())).bind_with_graceful_shutdown(
        ([0, 0, 0, 0], 3000),
        async {
            rx.await.ok();
        },
    );
    let server_task = tokio::spawn(server);

    // on ctrl+c, tell the server to shut down
    let err_ctrl_c = signal::ctrl_c().await;
    let _ = tx.send(());

    // wait for it to actually stop, then close the database connection
    let err_server_close = server_task.await;
    conn.close().await;

    // allow failures to be reported, in order, after graceful shutdown
    err_ctrl_c?;
    err_server_close?;
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Item {
    #[serde(default)]
    id: usize,
    name: String,
    description: String,
    #[serde(default)]
    count: usize,
    #[serde(default)]
    total_count: usize,
    #[serde(default)]
    last_wear: Option<DateTime<Utc>>,
    #[serde(default)]
    last_wash: Option<DateTime<Utc>>,
    #[serde(default = "utils::default_color")]
    color: String,
    #[serde(
        deserialize_with = "utils::split_comma",
        serialize_with = "utils::join_comma"
    )]
    tags: Vec<String>,
}

fn new_router(hb: Handlebars, db: Connection) -> warp::filters::BoxedFilter<(impl warp::Reply,)> {
    let hb = Arc::new(hb);
    let hbars = move |wt: WithTemplate<_>| wt.render(hb.clone());
    let with_state = warp::any().map(move || db.clone());

    let index = warp::get()
        .and(path::end())
        .and(warp::query::query())
        .and(with_state.clone())
        .and_then(home_page)
        .map(hbars.clone());

    let css = path("styles.css").and(path::end()).map(|| {
        warp::reply::with_header(
            include_str!("./static/styles.css"),
            "Content-Type",
            "text/css",
        )
    });

    let new = warp::get()
        .and(warp::path("new"))
        .and(path::end())
        .map(|| WithTemplate {
            name: "new",
            value: json!({}),
        })
        .map(hbars.clone());

    let post_item = warp::post()
        .and(path::end())
        .and(warp::body::content_length_limit(1024 * 32))
        .and(warp::body::form())
        .and(with_state.clone())
        .and_then(|item, conn: Connection| async move {
            conn.new_item(item).await.map_err(|e| {
                eprintln!("{}", e);
                warp::reject::not_found()
            })
        })
        .map(utils::go_home);

    let edit_item = warp::get()
        .and(path::param())
        .and(path::end())
        .and(with_state.clone())
        .and_then(handle_edit_form)
        .map(hbars);

    let update_item = warp::post()
        .and(path::param())
        .and(path::end())
        .and(warp::body::content_length_limit(1024 * 32))
        .and(warp::body::form())
        .and(with_state.clone())
        .and_then(|id, item, conn: Connection| async move {
            conn.update_item(Item { id, ..item }).await.map_err(|e| {
                eprintln!("{}", e);
                warp::reject::not_found()
            })
        })
        .map(utils::go_home);

    let increment_item = warp::post()
        .and(path::param())
        .and(warp::path("increment"))
        .and(path::end())
        .and(with_state.clone())
        .and_then(|id, conn: Connection| async move {
            conn.log_wear(id).await.map_err(|e| {
                eprintln!("{}", e);
                warp::reject::not_found()
            })
        })
        .map(utils::go_home);

    let reset_item = warp::post()
        .and(path::param())
        .and(warp::path("reset"))
        .and(path::end())
        .and(with_state.clone())
        .and_then(|id, conn: Connection| async move {
            conn.log_wash(id).await.map_err(|e| {
                eprintln!("{}", e);
                warp::reject::not_found()
            })
        })
        .map(utils::go_home);

    let delete_item = warp::post()
        .and(path::param())
        .and(path("remove"))
        .and(path::end())
        .and(with_state)
        .and_then(|id, conn: Connection| async move {
            conn.delete_item(id).await.map_err(|e| {
                eprintln!("{}", e);
                warp::reject::not_found()
            })
        })
        .map(utils::go_home);

    index
        .or(css)
        .or(warp::path("item").and(
            post_item
                .or(new)
                .or(edit_item)
                .or(update_item)
                .or(increment_item)
                .or(reset_item)
                .or(delete_item),
        ))
        .with(warp::log("wear"))
        .boxed()
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
enum SortItems {
    Name,
    Count,
    Wear,
    Wash,
}

#[derive(Deserialize)]
struct IndexOpts {
    sort: Option<SortItems>,
    descending: Option<bool>,
}

async fn home_page(
    params: IndexOpts,
    conn: Connection,
) -> Result<WithTemplate<serde_json::Value>, warp::Rejection> {
    let items = match conn
        .get_all(&params.sort, params.descending != Some(true))
        .await
    {
        Ok(i) => i
            .iter()
            .map(
                |Item {
                     id,
                     name,
                     description,
                     count,
                     total_count,
                     last_wear,
                     last_wash,
                     color,
                     tags,
                 }| {
                    json!({
                        "key": id,
                        "name": name,
                        "description": description,
                        "count": count,
                        "totalCount": total_count,
                        "hasWear": last_wear.is_some(),
                        "wear": last_wear,
                        "wearFmt": last_wear.map(|t| (t - Utc::now()).humanize()),
                        "hasWash": last_wash.is_some(),
                        "wash": last_wash,
                        "washFmt": last_wash.map(|t| (t - Utc::now()).humanize()),
                        "color": color,
                        "tags": tags.join(", "),
                    })
                },
            )
            .collect::<Vec<_>>(),

        Err(e) => {
            eprintln!("request for index: could not retrieve collection: {}", e);
            Vec::new()
        }
    };

    Ok(WithTemplate {
        name: "index",
        value: json!({
            "items": items,
            "numItems": items.len(),
            "sort": params.sort,
            "descending": params.descending,
        }),
    })
}

async fn handle_edit_form(
    id: usize,
    conn: Connection,
) -> Result<WithTemplate<serde_json::Value>, warp::Rejection> {
    match conn.get_item(id).await {
        Ok(Item {
            id,
            name,
            description,
            color,
            tags,
            ..
        }) => Ok(WithTemplate {
            name: "edit",
            value: json!({
                "edit": true,
                "key": id,
                "name": name,
                "description": description,
                "color": color,
                "tags": tags.join(", "),
            }),
        }),
        Err(e) => {
            eprintln!("{}", e);
            Err(warp::reject::not_found())
        }
    }
}
