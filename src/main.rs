use std::fs::File;
use std::io::Read;
use futures::{
    channel::mpsc::{channel, Receiver},
    SinkExt, StreamExt,
};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::Path;
use std::sync::Mutex;
use notify::event::{AccessKind, AccessMode};
use paris::{error, info};
use ureq_multipart::MultipartBuilder;
use serde::Deserialize;
use arboard::Clipboard;
use lazy_static::lazy_static;
use notify_rust::Notification;
use std::collections::HashMap;
use clap::Parser;

/// Simple program to greet a person
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    // Custom Domain or use the default one
    #[arg(short, long)]
    domain: String,

    // Path to the folder of screenshots
    #[arg(short, long)]
    path: String,

    // User ID
    #[arg(short, long)]
    uid: String,

    // Secret
    #[arg(short, long)]
    secret: String
}

pub struct KeyValueStore {
    store: HashMap<String, String>,
}

impl KeyValueStore {
    pub fn new() -> Self {
        KeyValueStore {
            store: HashMap::new(),
        }
    }

    pub fn set(&mut self, key: &str, value: &str) {
        self.store.insert(key.to_string(), value.to_string());
    }

    pub fn get(&self, key: &str) -> Option<&String> {
        self.store.get(key)
    }
}
lazy_static! {
    pub static ref CLIPBOARD: Mutex<Clipboard> = Mutex::new(Clipboard::new().unwrap());
    pub static ref CONFIG: Mutex<KeyValueStore> = Mutex::new(KeyValueStore::new());
}
#[derive(Deserialize)]
struct ResponseUpload {
    url: String,
}
const MAX_FILE_SIZE_BYTES: usize = 500 * 1024 * 1024;

/// Async, futures channel based event watching
fn main() {
    let args = Args::parse();

    let mut kv = CONFIG.lock().unwrap();

    kv.set("domain", &args.domain);
    kv.set("path", &args.path);
    kv.set("uid", &args.uid);
    kv.set("secret", &args.secret);

    let path = kv.get("path").unwrap();
    info!("Watching {}", path);

    futures::executor::block_on(async {
        if let Err(e) = async_watch(path).await {
            println!("error: {:?}", e)
        }
    });
}

fn async_watcher() -> notify::Result<(RecommendedWatcher, Receiver<notify::Result<Event>>)> {
    let (mut tx, rx) = channel(1);

    // Automatically select the best implementation for your platform.
    // You can also access each implementation directly e.g. INotifyWatcher.
    let watcher = RecommendedWatcher::new(
        move |res| {
            futures::executor::block_on(async {
                tx.send(res).await.unwrap();
            })
        },
        Config::default(),
    )?;

    Ok((watcher, rx))
}

async fn async_watch<P: AsRef<Path>>(path: P) -> notify::Result<()> {
    let (mut watcher, mut rx) = async_watcher()?;

    // Add a path to be watched. All files and directories at that path and
    // below will be monitored for changes.
    watcher.watch(path.as_ref(), RecursiveMode::Recursive)?;

    while let Some(res) = rx.next().await {
        match res {
            Ok(event) => handle_event(event).await,
            Err(e) => error!("CordX Upload error: {:?}", e),
        }
    }
    Ok(())
}


async fn handle_event(event: Event) -> () {
    match event.kind {
        EventKind::Access(AccessKind::Close(AccessMode::Write)) => {
            let mut clipboard = CLIPBOARD.lock().unwrap();
            for x in event.paths {
                let file_name = Path::new(&x)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("file_to_upload.txt")
                    .to_string();
                let mut file = match File::open(x.clone()) {
                    Ok(file) => file,
                    Err(err) => {
                        error!("Error Occurred while opening file: {:?}", err);
                        return;
                    }
                };

                let mut file_content = Vec::new();
                file.read_to_end(&mut file_content).expect("Error Occurred while opening file");

                if file_content.len() > MAX_FILE_SIZE_BYTES {
                    error!("File {:?} too big to be uploaded to CordX", file_name)
                }
                let (content_type,data) = MultipartBuilder::new()
                    .add_file("sharex", x.clone())
                    .unwrap().finish().unwrap();
                let config = CONFIG.lock().unwrap();
                let domain = config.get("domain").unwrap();
                let uid = config.get("uid").unwrap();
                let secret = config.get("secret").unwrap();
                let response =
                    ureq::post(format!("{}/api/upload/sharex", domain).as_str())
                    .set("userid", uid)
                    .set("secret", secret)
                    .set("Content-Type", &content_type)
                    .send_bytes(&data)
                        .map_err(|e| {
                            error!("Error while uploading files to CordX: {:?}", e)
                        });
                match response {
                    Ok(response) => {
                        if response.status() == 200 {
                            let data = response.into_json::<ResponseUpload>().unwrap();
                            clipboard.set_text(data.url.to_owned()).unwrap();
                            Notification::new()
                                .summary("CordX Upload")
                                .body("Picture Uploaded! URL Copied to Clipboard")
                                .show().expect("Panic: Notification Error");
                            info!("Successfully uploaded files: {:?}", data.url)
                        } else {
                            error!("Request failed with status code: {}", response.status());
                            // Handle other cases of failed request (e.g., unauthorized, server error, etc.)
                        }
                    }
                    Err(_e) => {
                        error!("Error while uploading files to CordX");
                    }
                }
            }
            },
        default => {
            info!("Change Occurred: {:?}", default)
        }
    }
    return ()
}