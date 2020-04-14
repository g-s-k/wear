#![deny(clippy::all)]

use {
    chrono::{DateTime, Utc},
    handlebars::Handlebars,
    serde::{Deserialize, Serialize},
    serde_json::json,
    sqlx::{
        prelude::*,
        sqlite::{SqlitePool, SqliteRow},
    },
    std::sync::Arc,
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
    #[serde(default)]
    id: usize,
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

impl<'c> FromRow<'c, SqliteRow<'c>> for Item {
    fn from_row(row: &SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(Item {
            id: row.try_get::<i32, _>("id")? as usize,
            name: row.try_get::<String, _>("name")?,
            description: row.try_get::<String, _>("description")?,
            count: row.try_get::<i32, _>("count")? as usize,
            last: row
                .try_get::<Option<&str>, _>("last")?
                .map(DateTime::parse_from_rfc3339)
                .map(Result::ok)
                .flatten()
                .map(|d| d.with_timezone(&Utc)),
            color: row.try_get::<String, _>("color")?,
            tags: row
                .try_get::<&str, _>("tags")?
                .split(',')
                .map(ToOwned::to_owned)
                .collect(),
        })
    }
}

async fn home_page(pool: SqlitePool) -> Result<WithTemplate<serde_json::Value>, warp::Rejection> {
    let items = match sqlx::query_as("SELECT * FROM garments")
        .fetch_all(&pool)
        .await
    {
        Ok(i) => {
            eprintln!("request for index: {} items found", i.len());
            i
        }
        Err(e) => {
            eprintln!("request for index: could not retrieve collection: {}", e);
            Vec::new()
        }
    };

    Ok(WithTemplate {
        name: "index",
        value: json!({
            "items": items.iter()
                        .map(
                            |Item {
                                id,
                                name,
                                description,
                                count,
                                last,
                                color,
                                tags,
                            }| {
                                json!({
                                    "key": id,
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
                        .collect::<Vec<_>>(),
            "numItems": items.len(),
            "user" : "warp"
        }),
    })
}

async fn handle_post_item(s: SqlitePool, item: Item) -> Result<impl warp::Reply, warp::Rejection> {
    match sqlx::query!(
        r#"
        INSERT INTO garments ( name, description, color, tags )
        VALUES ( ?, ?, ?, ? )
    "#,
        item.name,
        item.description,
        item.color,
        item.tags.join(",")
    )
    .execute(&s)
    .await
    {
        Ok(_) => Ok(utils::go_home()),
        Err(e) => {
            eprintln!("could not insert new item ({}): {}", item.name, e);
            Err(warp::reject::not_found())
        }
    }
}

async fn handle_update_item(
    i: usize,
    s: SqlitePool,
    Item {
        name,
        description,
        color,
        tags,
        ..
    }: Item,
) -> Result<impl warp::Reply, warp::Rejection> {
    match sqlx::query!(
        r#"
            UPDATE garments
            SET color = ?, name = ?, description = ?, tags = ?
            WHERE id = ?
        "#,
        color,
        name,
        description,
        tags.join(","),
        i as i32,
    )
    .execute(&s)
    .await
    {
        Ok(_) => Ok(utils::go_home()),
        Err(e) => {
            eprintln!("{}", e);
            Err(warp::reject::not_found())
        }
    }
}

async fn handle_increment(i: usize, s: SqlitePool) -> Result<impl warp::Reply, warp::Rejection> {
    match sqlx::query!(
        r#"
            UPDATE garments
            SET count = count + 1, last = ?
            WHERE id = ?
        "#,
        Utc::now().to_string(),
        i as i32,
    )
    .execute(&s)
    .await
    {
        Ok(_) => Ok(utils::go_home()),
        Err(e) => {
            eprintln!("{}", e);
            Err(warp::reject::not_found())
        }
    }
}

async fn handle_delete_item(i: usize, s: SqlitePool) -> Result<impl warp::Reply, warp::Rejection> {
    sqlx::query!("DELETE FROM garments WHERE id = ?", i as i32)
        .execute(&s)
        .await
        .map_err(|_| warp::reject::not_found())?;
    Ok(utils::go_home())
}

async fn handle_edit_form(
    i: usize,
    s: SqlitePool,
) -> Result<WithTemplate<serde_json::Value>, warp::Rejection> {
    match sqlx::query_as("SELECT * FROM garments WHERE id = ?")
        .bind(i as i32)
        .fetch_one(&s)
        .await
    {
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

    let db_path = format!(
        "sqlite://{}/data.db",
        std::env::current_dir().unwrap().to_string_lossy()
    );
    eprintln!("Connecting to database at {}", db_path);
    let state = SqlitePool::new(&db_path).await.unwrap();
    eprintln!(
        "Connected to database. Connection pool details: {:#?}",
        state
    );
    let with_state = warp::any().map(move || state.clone());

    let index = warp::get()
        .and(path::end())
        .and(with_state.clone())
        .and_then(home_page)
        .map(hbars.clone())
        .with(utils::html_header());

    let css = path("styles.css")
        .and(path::end())
        .map(|| include_str!("./static/styles.css"))
        .with(utils::css_header());

    let post_item = warp::post()
        .and(path::end())
        .and(with_state.clone())
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

    let router = index
        .or(css)
        .or(warp::path("item").and(
            post_item
                .or(edit_item)
                .or(update_item)
                .or(increment_item)
                .or(delete_item),
        ))
        .with(warp::log("wear"));

    warp::serve(router).run(([0, 0, 0, 0], 3000)).await;
}
