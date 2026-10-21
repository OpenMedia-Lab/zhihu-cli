mod api;
mod auth;
mod config;
mod http;

use clap::{Parser, Subcommand};

use crate::http::Session;

/// zhihu — a lightweight Zhihu CLI. JSON in, JSON out.
#[derive(Parser)]
#[command(name = "zhihu", version, about)]
struct Cli {
    /// Cookie header string, used instead of the saved session.
    #[arg(long, global = true)]
    cookie: Option<String>,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Log in: QR scan by default, --cookie to paste, --browser to extract.
    Login {
        /// Full Cookie header (or "-" to read from stdin).
        #[arg(long)]
        cookie: Option<String>,
        /// Extract zhihu.com cookies from an installed browser.
        #[arg(long)]
        browser: Option<Option<String>>,
    },
    /// Clear the saved session.
    Logout,
    /// Check auth: resolves /api/v4/me.
    Status,
    /// Show the current user's profile.
    Whoami,
    /// List the current account's articles.
    MyArticles {
        #[arg(long, default_value_t = 0)]
        offset: u32,
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// List a user's articles by url_token.
    UserArticles {
        url_token: String,
        #[arg(long, default_value_t = 0)]
        offset: u32,
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// List a user's answers by url_token.
    UserAnswers {
        url_token: String,
        #[arg(long, default_value_t = 0)]
        offset: u32,
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// List a user's pins (想法) by url_token.
    UserPins {
        url_token: String,
        #[arg(long, default_value_t = 0)]
        offset: u32,
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// Read an article, answer, question, or pin by id or URL.
    Read { id_or_url: String },
    /// Search Zhihu (general | topic | people).
    Search {
        keyword: String,
        #[arg(long, default_value = "general")]
        kind: String,
        #[arg(long, default_value_t = 10)]
        limit: u32,
    },
    /// Resolve a keyword to Zhihu topic ids (for --topic on article).
    Topics { keyword: String },
    /// Hot list.
    Hot {
        #[arg(long, default_value_t = 50)]
        limit: u32,
    },
    /// Upload an image; returns the zhimg src to embed. --url rehosts a remote image.
    Upload {
        path_or_url: String,
        /// Treat the argument as a remote URL zhihu fetches server-side.
        #[arg(long)]
        url: bool,
        #[arg(long, default_value = "article")]
        source: String,
    },
    /// Create an article — saves a DRAFT by default; --publish goes live.
    Article {
        title: String,
        /// HTML body. Use --content-file for a file instead.
        #[arg(long, conflicts_with = "content_file")]
        content: Option<String>,
        /// Read the HTML body from a file.
        #[arg(long)]
        content_file: Option<String>,
        /// Image path (repeatable); appended to the body.
        #[arg(short = 'i', long)]
        image: Vec<String>,
        /// Publish immediately instead of leaving a draft.
        #[arg(long)]
        publish: bool,
    },
    /// Publish a pin (想法). NOTE: pins have no draft mode — this goes live.
    Pin {
        title: String,
        #[arg(short = 'c', long)]
        content: Option<String>,
        #[arg(short = 'i', long)]
        image: Vec<String>,
    },
    /// Edit an existing draft or article. Omitted fields are preserved.
    EditArticle {
        id: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long, conflicts_with = "content_file")]
        content: Option<String>,
        #[arg(long)]
        content_file: Option<String>,
        /// Append uploaded images, preserving existing content when omitted.
        #[arg(short = 'i', long)]
        image: Vec<String>,
        #[arg(long)]
        publish: bool,
    },
    /// Read back an article draft (verify a draft-only post).
    Draft { id: String },
    /// Delete an article or draft by id.
    DeleteArticle { id: String },
    /// Delete a pin by id.
    DeletePin { id: String },
}

fn session(cli: &Cli) -> anyhow::Result<Session> {
    if let Some(raw) = &cli.cookie {
        let mut s = Session::new();
        for part in raw.split(';') {
            if let Some((k, v)) = part.trim().split_once('=') {
                s.cookies.insert(k.trim().into(), v.trim().into());
            }
        }
        return Ok(s);
    }
    Session::from_saved()
}

fn out(v: serde_json::Value) {
    println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
}

fn main() {
    let cli = Cli::parse();
    let r = run(&cli);
    if let Err(e) = r {
        eprintln!(
            "{}",
            serde_json::json!({"error": {"message": format!("{e:#}")}})
        );
        std::process::exit(1);
    }
}

fn run(cli: &Cli) -> anyhow::Result<()> {
    match &cli.cmd {
        Cmd::Login { cookie, browser } => {
            let cookie_owned = match cookie.as_deref() {
                Some("-") => Some(auth::read_cookie_stdin()?),
                other => other.map(str::to_string),
            };
            auth::login(
                cookie_owned.as_deref(),
                browser
                    .as_ref()
                    .map(|o| o.clone().unwrap_or_default())
                    .as_deref(),
            )
        }
        Cmd::Logout => {
            auth::logout();
            Ok(())
        }
        Cmd::Status => {
            let mut s = session(cli)?;
            let me = api::me(&mut s)?;
            out(serde_json::json!({
                "authenticated": true,
                "url_token": me.get("url_token"),
                "name": me.get("name"),
                "id": me.get("id"),
            }));
            Ok(())
        }
        Cmd::Whoami => {
            let mut s = session(cli)?;
            out(api::me(&mut s)?);
            Ok(())
        }
        Cmd::MyArticles { offset, limit } => {
            let mut s = session(cli)?;
            let me = api::me(&mut s)?;
            let token = me["url_token"].as_str().unwrap_or_default().to_string();
            out(api::user_articles(&mut s, &token, *offset, *limit)?);
            Ok(())
        }
        Cmd::UserArticles {
            url_token,
            offset,
            limit,
        } => {
            let mut s = session(cli)?;
            out(api::user_articles(&mut s, url_token, *offset, *limit)?);
            Ok(())
        }
        Cmd::UserAnswers {
            url_token,
            offset,
            limit,
        } => {
            let mut s = session(cli)?;
            out(api::user_answers(&mut s, url_token, *offset, *limit)?);
            Ok(())
        }
        Cmd::UserPins {
            url_token,
            offset,
            limit,
        } => {
            let mut s = session(cli)?;
            out(api::user_pins(&mut s, url_token, *offset, *limit)?);
            Ok(())
        }
        Cmd::Read { id_or_url } => {
            let mut s = session(cli)?;
            out(api::read(&mut s, id_or_url)?);
            Ok(())
        }
        Cmd::Search {
            keyword,
            kind,
            limit,
        } => {
            let mut s = session(cli)?;
            out(api::search(&mut s, keyword, kind, *limit)?);
            Ok(())
        }
        Cmd::Topics { keyword } => {
            let mut s = session(cli)?;
            out(api::topics(&mut s, keyword)?);
            Ok(())
        }
        Cmd::Hot { limit } => {
            let mut s = session(cli)?;
            out(api::hot(&mut s, *limit)?);
            Ok(())
        }
        Cmd::Upload {
            path_or_url,
            url,
            source,
        } => {
            let mut s = session(cli)?;
            if *url {
                out(api::upload_image_url(&mut s, path_or_url, source)?);
            } else {
                out(api::upload_image(&mut s, path_or_url, source)?);
            }
            Ok(())
        }
        Cmd::Article {
            title,
            content,
            content_file,
            image,
            publish,
        } => {
            let mut s = session(cli)?;
            let body = match (content, content_file) {
                (Some(c), _) => c.clone(),
                (None, Some(f)) => std::fs::read_to_string(f)?,
                (None, None) => String::new(),
            };
            out(api::create_article(
                &mut s,
                title,
                &body,
                &[],
                image,
                *publish,
            )?);
            Ok(())
        }
        Cmd::Pin {
            title,
            content,
            image,
        } => {
            let mut s = session(cli)?;
            out(api::create_pin(
                &mut s,
                title,
                content.as_deref().unwrap_or(""),
                image,
            )?);
            Ok(())
        }
        Cmd::EditArticle {
            id,
            title,
            content,
            content_file,
            image,
            publish,
        } => {
            let mut s = session(cli)?;
            let body = match (content, content_file) {
                (Some(c), _) => Some(c.clone()),
                (None, Some(f)) => Some(std::fs::read_to_string(f)?),
                _ => None,
            };
            out(api::edit_article(
                &mut s,
                id,
                title.as_deref(),
                body.as_deref(),
                image,
                *publish,
            )?);
            Ok(())
        }
        Cmd::Draft { id } => {
            let mut s = session(cli)?;
            out(api::read_draft(&mut s, id)?);
            Ok(())
        }
        Cmd::DeleteArticle { id } => {
            let mut s = session(cli)?;
            api::delete_article(&mut s, id)?;
            out(serde_json::json!({"id":id,"deleted":true}));
            Ok(())
        }
        Cmd::DeletePin { id } => {
            let mut s = session(cli)?;
            api::delete_pin(&mut s, id)?;
            out(serde_json::json!({"id":id,"deleted":true}));
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edit_accepts_partial_updates_and_rejects_conflicting_body_sources() {
        assert!(Cli::try_parse_from(["zhihu", "edit-article", "123", "--title", "new"]).is_ok());
        assert!(Cli::try_parse_from([
            "zhihu",
            "edit-article",
            "123",
            "--content",
            "",
            "--content-file",
            "x"
        ])
        .is_err());
    }
}
