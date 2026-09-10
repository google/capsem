//! The selected public HTTP contract. Every schema comes from a Rust wire type.

use crate::*;
use utoipa::openapi::path::{HttpMethod, OperationBuilder, ParameterBuilder, ParameterIn, PathItem, PathsBuilder};
use utoipa::openapi::request_body::RequestBodyBuilder;
use utoipa::openapi::schema::{ComponentsBuilder, KnownFormat, ObjectBuilder, SchemaFormat, Type};
use utoipa::openapi::security::{Http, HttpAuthScheme, SecurityRequirement, SecurityScheme};
use utoipa::openapi::{Content, ContentBuilder, Info, OpenApi, Ref, Required, ResponseBuilder};
use utoipa::{IntoParams, ToSchema};

/// Build the same contract served by the gateway and exported for SDK generation.
pub fn openapi() -> OpenApi {
    let mut doc = Document::default();
    doc.get::<HypervisorInfo>("/status", "getHypervisorInfo");
    doc.get::<ListResponse>("/vms/list", "listVms");
    doc.post::<ProvisionRequest, ProvisionResponse>("/vms/create", "createVm");
    doc.get::<SandboxInfo>("/vms/{id}/info", "getVmInfo");
    doc.get::<VmStatusResponse>("/vms/{id}/status", "getVmStatus");
    doc.post::<ExecRequest, ExecResponse>("/vms/{id}/exec", "execVm");
    doc.post::<ForkRequest, ForkResponse>("/vms/{id}/fork", "forkVm");
    doc.empty_post::<ProvisionResponse>("/vms/{id}/start", "startVm");
    doc.empty_post::<ProvisionResponse>("/vms/{id}/resume", "resumeVm");
    doc.logs();
    doc.get::<VmStatsSummaryResponse>("/vms/{id}/stats/summary", "getVmStatsSummary");
    doc.get::<SnapshotsStatus>("/vms/{id}/snapshots/status", "getVmSnapshotsStatus");
    doc.get::<SnapshotsList>("/vms/{id}/snapshots/list", "listVmSnapshots");
    let timeline = doc
        .operation::<TimelineResponse>("/vms/{id}/timeline", "getVmTimeline")
        .parameters(Some(TimelineQuery::into_params(|| Some(ParameterIn::Query))));
    doc.add("/vms/{id}/timeline", HttpMethod::Get, timeline);
    doc.schema::<HistoryLayerFilter>();
    let history = doc
        .operation::<HistoryResponse>("/vms/{id}/history", "getVmHistory")
        .parameters(Some(HistoryQuery::into_params(|| Some(ParameterIn::Query))));
    doc.add("/vms/{id}/history", HttpMethod::Get, history);
    let changes = doc
        .operation::<ChangesResponse>("/vms/{id}/changes", "getVmChanges")
        .parameters(Some(ChangesQuery::into_params(|| Some(ParameterIn::Query))));
    doc.add("/vms/{id}/changes", HttpMethod::Get, changes);
    doc.get::<ProfilesListResponse>("/profiles/list", "listProfiles");
    doc.get::<UpdateStatusResponse>("/update/status", "getUpdateStatus");
    doc.post::<UpdateApplyRequest, UpdateActionResponse>("/update/apply", "updateHypervisor");
    doc.files();
    doc.finish()
}

#[derive(Default)]
struct Document {
    paths: PathsBuilder,
    components: ComponentsBuilder,
}

impl Document {
    fn schema<T: ToSchema>(&mut self) -> Ref {
        let mut nested = Vec::new();
        T::schemas(&mut nested);
        self.components = std::mem::take(&mut self.components)
            .schema_from::<T>()
            .schemas_from_iter(nested);
        Ref::from_schema_name(T::name())
    }

    fn operation<T: ToSchema>(&mut self, path: &str, id: &str) -> OperationBuilder {
        let result = self.schema::<T>();
        let error = self.schema::<ErrorResponse>();
        let mut operation = OperationBuilder::new()
            .operation_id(Some(id))
            .response(
                "200",
                ResponseBuilder::new()
                    .description("Success")
                    .content("application/json", Content::new(Some(result))),
            )
            .response(
                "default",
                ResponseBuilder::new()
                    .description("HTTP error; inspect status and error body")
                    .content("application/json", Content::new(Some(error)))
                    .content(
                        "text/plain",
                        Content::new(Some(ObjectBuilder::new().schema_type(Type::String))),
                    ),
            );
        if path.contains("{id}") {
            operation = operation.parameter(
                ParameterBuilder::new()
                    .name("id")
                    .parameter_in(ParameterIn::Path)
                    .required(Required::True)
                    .schema(Some(ObjectBuilder::new().schema_type(Type::String))),
            );
        }
        operation
    }

    fn add(&mut self, path: &str, method: HttpMethod, operation: OperationBuilder) {
        self.paths = std::mem::take(&mut self.paths).path(path, PathItem::new(method, operation.build()));
    }

    fn get<T: ToSchema>(&mut self, path: &str, id: &str) {
        let operation = self.operation::<T>(path, id);
        self.add(path, HttpMethod::Get, operation);
    }

    fn post<Q: ToSchema, T: ToSchema>(&mut self, path: &str, id: &str) {
        let request = self.schema::<Q>();
        let operation = self.operation::<T>(path, id).request_body(Some(
            RequestBodyBuilder::new()
                .required(Some(Required::True))
                .content("application/json", Content::new(Some(request)))
                .build(),
        ));
        self.add(path, HttpMethod::Post, operation);
    }

    fn empty_post<T: ToSchema>(&mut self, path: &str, id: &str) {
        let operation = self.operation::<T>(path, id);
        self.add(path, HttpMethod::Post, operation);
    }

    fn logs(&mut self) {
        let path = "/vms/{id}/logs";
        let operation = self
            .operation::<LogsResponse>(path, "getVmLogs")
            .parameters(Some(LogQuery::into_params(|| Some(ParameterIn::Query))));
        self.add(path, HttpMethod::Get, operation);

        let path = "/host-logs/{name}";
        let source = self.schema::<HostLogSource>();
        let result = self.schema::<HostLogsResponse>();
        let operation = self
            .operation::<HostLogsResponse>(path, "getHypervisorLogs")
            .description(Some(
                "Send Accept: application/json for the typed response. Without it the route returns plain text.",
            ))
            .parameter(
                ParameterBuilder::new()
                    .name("name")
                    .parameter_in(ParameterIn::Path)
                    .required(Required::True)
                    .schema(Some(source)),
            )
            .parameters(Some(LogQuery::into_params(|| Some(ParameterIn::Query))))
            .response(
                "200",
                ResponseBuilder::new()
                    .description("Bounded, filtered log tail")
                    .content("application/json", Content::new(Some(result)))
                    .content(
                        "text/plain",
                        Content::new(Some(ObjectBuilder::new().schema_type(Type::String))),
                    ),
            );
        self.add(path, HttpMethod::Get, operation);
    }

    fn files(&mut self) {
        let path = "/vms/{id}/files/list";
        let operation = self
            .operation::<FileListResponse>(path, "listVmFiles")
            .parameter(
                ParameterBuilder::new()
                    .name("path")
                    .parameter_in(ParameterIn::Query)
                    .schema(Some(ObjectBuilder::new().schema_type(Type::String))),
            )
            .parameter(
                ParameterBuilder::new()
                    .name("depth")
                    .parameter_in(ParameterIn::Query)
                    .schema(Some(ObjectBuilder::new().schema_type(Type::Integer))),
            );
        self.add(path, HttpMethod::Get, operation);

        let path = "/vms/{id}/files/content";
        let query = ParameterBuilder::new()
            .name("path")
            .parameter_in(ParameterIn::Query)
            .required(Required::True)
            .schema(Some(ObjectBuilder::new().schema_type(Type::String)))
            .build();
        let binary = ContentBuilder::new()
            .schema(Some(
                ObjectBuilder::new()
                    .schema_type(Type::String)
                    .format(Some(SchemaFormat::KnownFormat(KnownFormat::Binary))),
            ))
            .build();
        let download = self
            .operation::<UploadResponse>(path, "downloadVmFile")
            .parameter(query.clone())
            .response(
                "200",
                ResponseBuilder::new()
                    .description("File bytes")
                    .content("application/octet-stream", binary.clone()),
            );
        self.add(path, HttpMethod::Get, download);
        let upload = self
            .operation::<UploadResponse>(path, "uploadVmFile")
            .parameter(query)
            .request_body(Some(
                RequestBodyBuilder::new()
                    .required(Some(Required::True))
                    .content("application/octet-stream", binary)
                    .build(),
            ));
        self.add(path, HttpMethod::Post, upload);
    }

    fn finish(self) -> OpenApi {
        let mut api = OpenApi::new(
            Info::new("Capsem Gateway", env!("CARGO_PKG_VERSION")),
            self.paths.build(),
        );
        api.components = Some(
            self.components
                .security_scheme("bearerAuth", SecurityScheme::Http(Http::new(HttpAuthScheme::Bearer)))
                .build(),
        );
        api.security = Some(vec![SecurityRequirement::new("bearerAuth", Vec::<String>::new())]);
        api
    }
}
