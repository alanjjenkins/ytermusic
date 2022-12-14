use std::path::PathBuf;
use std::str::FromStr;

use ytpapi::YTApi;

fn main() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let api = YTApi::from_header_file(PathBuf::from_str("headers.txt").unwrap().as_path())
                .await
                .unwrap();
            api.search("Carpenter Brut")
                .await
                .iter()
                .for_each(|playlist| {
                    println!("{:?}", playlist);
                });
        });
}
