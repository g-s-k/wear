use {
    super::{Item, SortItems},
    chrono::{DateTime, Utc},
    directories::ProjectDirs,
    sqlx::{
        prelude::*,
        sqlite::{SqlitePool, SqliteRow},
    },
    std::{
        ffi::OsString,
        fmt::{self, Display},
        fs,
        io::ErrorKind,
        path::PathBuf,
        time::Instant,
    },
};

type ExecResult = sqlx::Result<u64>;

impl<'c> FromRow<'c, SqliteRow<'c>> for Item {
    fn from_row(row: &SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(Item {
            id: row.try_get::<i32, _>("id")? as usize,
            name: row.try_get::<String, _>("name")?,
            description: row.try_get::<String, _>("description")?,
            count: row.try_get::<i32, _>("count")? as usize,
            total_count: row.try_get::<i32, _>("total")? as usize,
            last_wash: row
                .try_get::<Option<&str>, _>("wash")?
                .map(DateTime::parse_from_rfc3339)
                .map(Result::ok)
                .flatten()
                .map(|d| d.with_timezone(&Utc)),
            last_wear: row
                .try_get::<Option<&str>, _>("wear")?
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

#[derive(Debug)]
pub enum ConnectionError {
    Utf8(OsString),
}

impl std::error::Error for ConnectionError {}

impl Display for ConnectionError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Utf8(s) => write!(
                f,
                "Cannot convert the following raw path to UTF-8: {}",
                s.to_string_lossy()
            ),
        }
    }
}

#[derive(Clone)]
pub(crate) struct Connection(SqlitePool);

impl Connection {
    pub(crate) async fn new(data_path: Option<PathBuf>) -> anyhow::Result<Self> {
        const PROTOCOL: &str = "sqlite://";
        const QUALIFIER: &str = "xyz.georgekaplan";
        const ORG: &str = "g-s-k";
        const APP_NAME: &str = "wear";
        const DEFAULT_FILE_NAME: &str = "data.db";

        let mut directory;
        let mut file_name = OsString::from(DEFAULT_FILE_NAME);

        if let Some(p) = data_path {
            directory = p.clone();

            match fs::metadata(&p) {
                // if the specified path exists and is a directory, use it with the default filename
                Ok(m) if m.is_dir() => (),

                // if it's a file, try splitting off the filename
                Ok(m) if m.is_file() => {
                    eprintln!("{:?}", m);
                    if let (Some(d), Some(f)) = (p.parent(), p.file_name()) {
                        file_name = f.to_os_string();
                        directory = d.to_path_buf();
                    }
                }

                // if it's something else, hmm...
                Ok(_) => (),

                // if it doesn't exist yet...
                Err(e) if e.kind() == ErrorKind::NotFound => {
                    eprintln!("{:?}", e);
                    // and it has a file extension, use it in its entirety
                    if let (Some(d), Some(f), Some(_)) = (p.parent(), p.file_name(), p.extension())
                    {
                        file_name = f.to_os_string();
                        directory = d.to_path_buf();
                    }
                }

                // otherwise, get the heck out of here
                Err(other) => return Err(other.into()),
            }
        } else if let Some(p_dirs) = ProjectDirs::from(QUALIFIER, ORG, APP_NAME) {
            directory = p_dirs.data_dir().to_path_buf();
        } else {
            eprintln!("Could not determine a platform-appropriate location for data storage. Using the current directory.");
            directory = std::env::current_dir()?;
        };

        fs::create_dir_all(&directory)?;

        directory.push(file_name);
        let mut db_path = OsString::from(PROTOCOL);
        db_path.push(directory);

        let string_path = db_path.into_string().map_err(ConnectionError::Utf8)?;

        eprintln!("Connecting to database at {}", string_path);
        let before = Instant::now();

        let pool = SqlitePool::new(&string_path).await?;

        eprintln!(
            "Connected to database after {}µs. Connection pool details: {:#?}",
            before.elapsed().as_micros(),
            pool
        );

        Ok(Self(pool))
    }

    pub(crate) async fn close(&self) {
        eprintln!(
            "\r\nClosing database connection [{} connection(s), {} idle]",
            self.0.size(),
            self.0.idle()
        );
        let before = Instant::now();

        self.0.close().await;

        eprintln!(
            "Database connection closed after {}µs",
            before.elapsed().as_micros()
        );
    }

    pub(crate) async fn get_all(
        &self,
        order: &Option<SortItems>,
        mut ascending: bool,
    ) -> sqlx::Result<Vec<Item>> {
        let mut cmd = "SELECT * FROM garments".to_string();

        if let Some(column) = order {
            cmd += " ORDER BY ";
            cmd += match column {
                SortItems::Name => "name",
                SortItems::Count => "count",

                // values stored as datetimes are (to the user) in reverse sort order
                SortItems::Wear => {
                    ascending ^= true;
                    "datetime(wear)"
                }
                SortItems::Wash => {
                    ascending ^= true;
                    "datetime(wash)"
                }
            };
            cmd += if ascending { " ASC" } else { " DESC" };
        }

        sqlx::query_as(&cmd).fetch_all(&self.0).await
    }

    pub(crate) async fn new_item(
        &self,
        Item {
            name,
            description,
            color,
            tags,
            ..
        }: Item,
    ) -> ExecResult {
        sqlx::query("INSERT INTO garments ( name, description, color, tags ) VALUES ( ?, ?, ?, ? )")
            .bind(name)
            .bind(description)
            .bind(color)
            .bind(tags.join(","))
            .execute(&self.0)
            .await
    }

    pub(crate) async fn get_item(&self, item_id: usize) -> sqlx::Result<Item> {
        sqlx::query_as("SELECT * FROM garments WHERE id = ?")
            .bind(item_id as i32)
            .fetch_one(&self.0)
            .await
    }

    pub(crate) async fn update_item(
        &self,
        Item {
            id,
            name,
            description,
            color,
            tags,
            ..
        }: Item,
    ) -> ExecResult {
        sqlx::query(
            r#"
            UPDATE garments
            SET color = ?, name = ?, description = ?, tags = ?
            WHERE id = ?
        "#,
        )
        .bind(color)
        .bind(name)
        .bind(description)
        .bind(tags.join(","))
        .bind(id as i32)
        .execute(&self.0)
        .await
    }

    pub(crate) async fn delete_item(&self, item_id: usize) -> ExecResult {
        sqlx::query("DELETE FROM garments WHERE id = ?")
            .bind(item_id as i32)
            .execute(&self.0)
            .await
    }

    pub(crate) async fn log_wear(&self, item_id: usize) -> ExecResult {
        sqlx::query(
            "UPDATE garments SET count = count + 1, total = total + 1, wear = ? WHERE id = ?",
        )
        .bind(Utc::now().to_rfc3339())
        .bind(item_id as i32)
        .execute(&self.0)
        .await
    }

    pub(crate) async fn log_wash(&self, item_id: usize) -> ExecResult {
        sqlx::query("UPDATE garments SET count = 0, wash = ? WHERE id = ?")
            .bind(Utc::now().to_rfc3339())
            .bind(item_id as i32)
            .execute(&self.0)
            .await
    }
}
