//! Catalog-selected package details. Registry/cache verification belongs to assets.
use super::*;
use axum::extract::Path as RoutePath;
use capsem_api::{ImageInventoryQuery, ImageInventoryResponse, ImagePackage, ImagePackageInventory, ImageTool};

pub(crate) type InventoryFuture =
    Pin<Box<dyn Future<Output = anyhow::Result<capsem_assets::oci::PackageInventory>> + Send>>;

pub(crate) async fn handle_inventory(
    State(state): State<Arc<ServiceState>>,
    RoutePath(name): RoutePath<String>,
    Query(query): Query<ImageInventoryQuery>,
) -> Result<Json<ImageInventoryResponse>, AppError> {
    tokio::time::timeout(Duration::from_secs(30), read(&state, name, query))
        .await
        .map_err(|_| {
            AppError::new(
                StatusCode::GATEWAY_TIMEOUT,
                "image inventory read exceeded 30 seconds".into(),
            )
            .with_timeout_secs(30)
        })?
}

async fn read(
    state: &ServiceState,
    name: String,
    query: ImageInventoryQuery,
) -> Result<Json<ImageInventoryResponse>, AppError> {
    if !is_catalog_name(&name) {
        return Err(ImageError::Invalid("inventory requires a catalog name".into()).into());
    }
    let mut decision = Decision::new(state).await?;
    let requested = decision.resolve(&name).await?;
    if requested.pull != query.image {
        return Err(AppError::new(
            StatusCode::CONFLICT,
            "selected image changed; reload the catalog".into(),
        ));
    }
    decision.check_source(&requested).await?;
    let architecture = stage::catalog_architecture().map_err(|e| ImageError::Failed(format!("{e:#}")))?;
    let loaded = decision
        .load(false)
        .await
        .cloned()
        .ok_or_else(|| ImageError::Failed("image catalog unavailable".into()))?;
    let selected = loaded
        .catalog
        .resolve(&name, architecture, RUNTIME_CONTRACT)
        .map_err(|e| ImageError::Invalid(format!("{e:#}")))?;
    decision
        .policy
        .admit(&selected)
        .map_err(|e| ImageError::Refused(format!("{e:#}")))?;
    let version = loaded
        .catalog
        .entry(&name)
        .and_then(|entry| entry.versions().iter().find(|v| v.image() == &selected))
        .ok_or_else(|| ImageError::Failed("selected catalog version unavailable".into()))?;
    let mut response = ImageInventoryResponse {
        image: query.image,
        architecture: architecture.as_str().into(),
        artifact_reference: None,
        inventory: None,
    };
    let Some(evidence) = version
        .evidence(architecture)
        .iter()
        .find(|e| e.kind == "package_inventory")
    else {
        return Ok(Json(response));
    };
    if evidence.media_type != "application/vnd.capsem.package-inventory.v1+json" {
        return Err(ImageError::Failed("unsupported catalog package inventory media type".into()).into());
    }
    let reference = evidence.reference.to_string();
    let artifact = super::requested(&reference)?;
    if artifact.identity.repository() != requested.identity.repository() {
        return Err(ImageError::Refused("inventory must share the selected image repository".into()).into());
    }
    decision.check_source(&artifact).await?;
    let inventory = state
        .containers
        .source
        .fetch_inventory(loaded.source, requested.pull, reference.clone(), decision.parent)
        .await
        .map_err(|e| ImageError::Failed(format!("read package inventory: {e:#}")))?;
    response.artifact_reference = Some(reference);
    response.inventory = Some(ImagePackageInventory {
        schema: inventory.schema,
        subject: inventory.subject,
        architecture: inventory.architecture,
        packages: inventory
            .packages
            .into_iter()
            .map(|p| ImagePackage {
                name: p.name,
                version: p.version,
                architecture: p.architecture,
            })
            .collect(),
        tools: inventory
            .tools
            .into_iter()
            .map(|p| ImageTool {
                name: p.name,
                version: p.version,
            })
            .collect(),
    });
    Ok(Json(response))
}
