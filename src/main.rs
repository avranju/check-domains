use std::{env, fmt::Display};

use anyhow::Result;
use futures_util::future::join_all;
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use tokio::{net::TcpStream, sync::mpsc};
use tokio_native_tls::{native_tls, TlsConnector};

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Domain {
    #[serde(rename = "Rank")]
    rank: usize,

    #[serde(rename = "Domain")]
    domain: String,

    #[serde(rename = "Open Page Rank")]
    page_rank: f64,
}

impl Display for Domain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Domain: {}, Rank = {}, Open Page Rank = {}",
            self.domain, self.rank, self.page_rank
        )
    }
}

const WORKERS: usize = 50;
const CHANNEL_BUFFER: usize = 10;

#[tokio::main]
async fn main() -> Result<()> {
    let mut channels = vec![];
    channels.resize_with(WORKERS, || mpsc::channel::<Domain>(CHANNEL_BUFFER));
    let workers = channels
        .into_iter()
        .enumerate()
        .map(|(index, (tx, rx))| (tx, tokio::spawn(worker_tls(index, rx))))
        .collect::<Vec<_>>();

    if let Some(file_name) = env::args().nth(1) {
        let mut rdr = csv::Reader::from_path(file_name)?;

        for (index, result) in rdr.deserialize().enumerate() {
            let (tx, _) = &workers[index % WORKERS];
            tx.send(result?).await?;
        }
    } else {
        usage();
    }

    // wait for the workers to do their thing
    let _ = join_all(workers.into_iter().map(|(_, w)| w)).await;

    Ok(())
}

fn usage() {
    eprintln!("Usage:\n  check-domains <<domains.csv>>");
}

async fn worker_tls(index: usize, mut rx: mpsc::Receiver<Domain>) -> Result<()> {
    while let Some(domain) = rx.recv().await {
        let host = format!("{}:443", domain.domain);
        let stream = TcpStream::connect(&host).await?;
        let connector = TlsConnector::from(native_tls::TlsConnector::new()?);
        match connector
            .connect(&domain.domain, stream)
            .await
            .map_err(|e| e.to_string())
        {
            Ok(_) => println!("[{index}] ok {}", domain),
            Err(err) if err.contains("EOF") => eprintln!("[{index}] BLOCKED! domain {}", domain),
            Err(err) => eprintln!("ERR! domain {}, {}", domain, err),
        }
    }

    Ok(())
}

async fn _worker_http(index: usize, mut rx: mpsc::Receiver<Domain>) -> Result<()> {
    let client = Client::new();

    while let Some(domain) = rx.recv().await {
        let url = Url::parse(&format!("https://{}", domain.domain))?;
        match client.head(url).send().await.map_err(|e| e.to_string()) {
            Ok(res) => println!(
                "[{index}] checked {}, status code: {}",
                domain,
                res.status()
            ),
            Err(err) if err.contains("EOF") => eprintln!("[{index}] BLOCKED! domain {}", domain),
            Err(err) => eprintln!("[{index}] ERR! domain {}, {}", domain, err),
        }
    }

    Ok(())
}
