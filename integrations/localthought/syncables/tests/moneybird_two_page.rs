//! Offline regression for the pinned Moneybird contact catalog and Link pagination.
//! Fixtures: localthought/openapi-directory c4a7e561, APIs/moneybird.com/v2-readonly;
//! localthought/overlays e00543f9, moneybird.com/api/v2. Keep these in sync with
//! integration-proxy's catalog pins when updating the integration.
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use indexmap::IndexMap;
use syncables::client::client::{Fetch, HttpRequest, HttpResponse};
use syncables::{
    derive_ontology, discover_resource_model, load_open_api_document_with_overlays, ClientConfig,
    Credentials, InMemoryStorage, Storage, SyncClient,
};

struct TwoPages(Arc<Mutex<Vec<String>>>);

#[async_trait]
impl Fetch for TwoPages {
    async fn fetch(&self, request: HttpRequest) -> syncables::Result<HttpResponse> {
        assert_eq!(request.method, "GET");
        self.0.lock().unwrap().push(request.url.clone());
        let body = if request.url.contains("page=2") {
            r#"[{"id":"2","company_name":"Second"}]"#
        } else {
            r#"[{"id":"1","company_name":"First"}]"#
        };
        let mut headers = IndexMap::new();
        if !request.url.contains("page=2") {
            headers.insert(
                "Link".into(),
                "<https://moneybird.com/api/v2/123/contacts.json?page=2>; rel=\"next\"".into(),
            );
        }
        Ok(HttpResponse {
            status: 200,
            headers,
            body: body.as_bytes().to_vec(),
        })
    }
}

#[tokio::test]
async fn moneybird_contacts_fetch_two_pages() {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/moneybird");
    let document_path = fixtures.join("openapi.yaml");
    let overlays = vec![
        fixtures.join("auth-overlay.yaml"),
        fixtures.join("pagination-overlay.yaml"),
        fixtures.join("crud-causality-overlay.yaml"),
    ];
    let document = load_open_api_document_with_overlays(document_path.to_str().unwrap(), &overlays)
        .await
        .unwrap();
    let model = discover_resource_model(&document).unwrap();
    assert_eq!(model.collections.len(), 1);
    assert_eq!(model.collections[0].name, "contacts");
    assert_eq!(model.collections[0].context_params, ["administration_id"]);
    let ontology = derive_ontology(&document).unwrap();
    assert!(
        ontology
            .terms
            .iter()
            .any(|term| term.shortname == "company-name"),
        "contact fields must survive schema resolution: {ontology:?}"
    );
    let urls = Arc::new(Mutex::new(Vec::new()));
    let client = SyncClient::new(
        ClientConfig {
            document: document_path,
            overlays,
            credentials: Credentials::Bearer("test-token".into()),
            constants: BTreeMap::from([("administration_id".into(), "123".into())]),
            ontology_base_url: "https://atomicdata.dev/integrations/moneybird".into(),
        },
        Arc::new(TwoPages(urls.clone())),
    )
    .unwrap();
    let storage = InMemoryStorage::new();
    let report = client.sync(&storage).await.unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(report.read.get("contact"), Some(&2));
    assert_eq!(storage.list("123", "contact").await.unwrap().len(), 2);
    assert_eq!(
        urls.lock().unwrap().as_slice(),
        [
            "https://moneybird.com/api/v2/123/contacts.json",
            "https://moneybird.com/api/v2/123/contacts.json?page=2",
        ]
    );
}
