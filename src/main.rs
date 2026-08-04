use std::{
	collections::{HashMap, HashSet},
	fs::File,
	io::{Error, Read, Write},
};

use serde::{Deserialize, Serialize};


#[derive(Debug, Deserialize, Clone, Eq, PartialEq, Hash, Serialize)]
struct AssetManifest {
	hash:  String,
	bytes: usize,
}
#[derive(Debug, Deserialize, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Record {
	id:             String,
	name:           Option<String>,
	path:           Option<String>,
	asset_manifest: Vec<AssetManifest>,
}

#[derive(Debug, Deserialize, Clone, Serialize)]
struct ExportRecord {
	id:   String,
	name: Option<String>,
	path: Option<String>,
}
#[derive(Debug, Deserialize, Clone, Serialize)]
struct ExportAssetManifest {
	asset:   AssetManifest,
	records: Vec<ExportRecord>,
}


#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResoniteApiResponse {
	_asset_hash:  String,
	_bytes:       u64,
	free:         bool,
	_is_uploaded: bool,
	_row_key:     String,
	//eTag Unknown what type this should be.
}


fn get_asset_list(path: &str) -> HashSet<String> {
	fn get_asset_list_internal(path: &str) -> Result<HashSet<String>, Error> {
		let file = File::open(path)?;
		Ok(serde_json::de::from_reader(file)?)
	}
	get_asset_list_internal(path).unwrap_or_default()
}

fn write_asset_list(path: &str, list: HashSet<String>) {
	let mut list: Vec<String> = list.into_iter().collect();
	list.sort();
	let list_json = serde_json::to_string_pretty(&list).unwrap();
	File::create(path).unwrap().write_all(list_json.as_bytes()).unwrap();
}

#[tokio::main]
async fn main() {
	let mut records_string = String::new();
	File::open("./Records.json").unwrap().read_to_string(&mut records_string).unwrap();
	let records: Vec<Record> = serde_json::from_str(&records_string).unwrap();

	let mut free_assets: HashSet<String> = get_asset_list("./free_assets.json");
	let mut non_free_assets: HashSet<String> = get_asset_list("./non_free_assets.json");

	let records: Vec<_> = records
		.into_iter()
		.map(|mut record| {
			record.asset_manifest.retain(|asset| !free_assets.contains(&asset.hash));
			record
		})
		.collect();


	let mut map_by_asset_hashes: HashMap<String, Vec<_>> = HashMap::new();
	let mut bytes_by_asset_hash: HashMap<String, usize> = HashMap::new();

	for record in records {
		let record_clone = record.clone();
		for asset in record.asset_manifest {
			map_by_asset_hashes.entry(asset.hash.clone()).or_default().push(record_clone.clone());
			bytes_by_asset_hash.insert(asset.hash, asset.bytes);
		}
	}


	let (mut existing_non_free_assets, unknown_assets): (Vec<_>, Vec<_>) = map_by_asset_hashes
		.keys()
		.map(|key| key.to_owned())
		.partition(|asset| non_free_assets.contains(asset));

	println!("Fetching {} assets", unknown_assets.len());

	let client = match reqwest::Client::builder().pool_max_idle_per_host(1).build() {
		Ok(client) => client,
		Err(err) => {
			eprintln!("Error in reqwest: {err}");
			std::process::exit(1)
		},
	};

	for asset in unknown_assets {
		// Fetch from https://api.resonite.com/assets/{Hash} and check for free property
		let url = format!("https://api.resonite.com/assets/{asset}");
		println!("Fetching from {url}");
		match client.get(&url).send().await {
			Ok(response) => match response.json::<ResoniteApiResponse>().await {
				Ok(ResoniteApiResponse { free: true, .. }) => {
					free_assets.insert(asset);
				},
				Ok(ResoniteApiResponse { free: false, .. }) => {
					non_free_assets.insert(asset.clone());
					existing_non_free_assets.push(asset);
				},
				Err(err) => {
					eprintln!("Error fetching from {url}: {err}");
				},
			},
			Err(err) => {
				eprintln!("Error fetching from {url}: {err}");
			},
		};
	}

	let mut exists_list: Vec<_> = existing_non_free_assets
		.iter()
		.map(|hash| AssetManifest { hash: hash.clone(), bytes: *bytes_by_asset_hash.get(hash).unwrap() })
		.collect();
	exists_list.sort_by(|a, b| {
		let a_len = map_by_asset_hashes.get(&a.hash).unwrap().len();
		let b_len = map_by_asset_hashes.get(&b.hash).unwrap().len();
		let cmp = b_len.cmp(&a_len);
		if cmp.is_eq() { a.bytes.cmp(&b.bytes) } else { cmp }
	});

	let export_manifests: Vec<ExportAssetManifest> = exists_list
		.iter()
		.map(|asset| {
			let records = map_by_asset_hashes.get(&asset.hash).unwrap();
			ExportAssetManifest {
				asset:   asset.clone(),
				records: records
					.iter()
					.map(|record| ExportRecord { id: record.id.clone(), name: record.name.clone(), path: record.path.clone() })
					.collect(),
			}
		})
		.collect();
	let mut export_file = File::create("./exportedReport.json").unwrap();
	let _ = export_file.write(serde_json::to_string_pretty(&export_manifests).unwrap().as_bytes()).unwrap();

	for key in &exists_list {
		let records: String = map_by_asset_hashes
			.get(&key.hash)
			.unwrap()
			.iter()
			.map(|record| {
				format!(
					"    {}, {}, {}",
					record.id,
					record.name.as_deref().unwrap_or("no_name").to_owned(),
					record.path.as_deref().unwrap_or("no_path").to_owned()
				)
			})
			.collect::<Vec<String>>()
			.join("\n");

		println!("{} {}={}MB: \n{}\n", key.hash, key.bytes, key.bytes / (1024 * 1024), records);
	}

	println!("total_exists: {}", exists_list.iter().fold(0, |acc, val| acc + val.bytes));
	write_asset_list("./free_assets.json", free_assets);
	write_asset_list("./non_free_assets.json", non_free_assets);
}
