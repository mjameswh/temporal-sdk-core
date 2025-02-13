//! Worker-specific client needs

pub(crate) mod mocks;

use temporal_client::{Client, RetryClient, WorkflowService};
use temporal_sdk_core_protos::{
    coresdk::workflow_commands::QueryResult,
    temporal::api::{
        command::v1::Command,
        common::v1::{
            MeteringMetadata, Payloads, WorkerVersionCapabilities, WorkerVersionStamp,
            WorkflowExecution,
        },
        enums::v1::{TaskQueueKind, WorkflowTaskFailedCause},
        failure::v1::Failure,
        query::v1::WorkflowQueryResult,
        sdk::v1::WorkflowTaskCompletedMetadata,
        taskqueue::v1::{StickyExecutionAttributes, TaskQueue, TaskQueueMetadata},
        workflowservice::v1::{get_system_info_response::Capabilities, *},
    },
    TaskToken,
};

type Result<T, E = tonic::Status> = std::result::Result<T, E>;

/// Contains everything a worker needs to interact with the server
pub(crate) struct WorkerClientBag {
    client: RetryClient<Client>,
    namespace: String,
    identity: String,
    worker_build_id: String,
    use_versioning: bool,
}

impl WorkerClientBag {
    pub fn new(
        client: RetryClient<Client>,
        namespace: String,
        identity: String,
        worker_build_id: String,
        use_versioning: bool,
    ) -> Self {
        Self {
            client,
            namespace,
            identity,
            worker_build_id,
            use_versioning,
        }
    }

    fn default_capabilities(&self) -> Capabilities {
        self.capabilities().cloned().unwrap_or_default()
    }

    fn binary_checksum(&self) -> String {
        if self.default_capabilities().build_id_based_versioning {
            "".to_string()
        } else {
            self.worker_build_id.clone()
        }
    }

    fn worker_version_capabilities(&self) -> Option<WorkerVersionCapabilities> {
        if self.default_capabilities().build_id_based_versioning {
            Some(WorkerVersionCapabilities {
                build_id: self.worker_build_id.clone(),
                use_versioning: self.use_versioning,
            })
        } else {
            None
        }
    }

    fn worker_version_stamp(&self) -> Option<WorkerVersionStamp> {
        if self.default_capabilities().build_id_based_versioning {
            Some(WorkerVersionStamp {
                build_id: self.worker_build_id.clone(),
                bundle_id: "".to_string(),
                use_versioning: self.use_versioning,
            })
        } else {
            None
        }
    }
}

/// This trait contains everything workers need to interact with Temporal, and hence provides a
/// minimal mocking surface. Delegates to [WorkflowClientTrait] so see that for details.
#[cfg_attr(test, mockall::automock)]
#[async_trait::async_trait]
pub(crate) trait WorkerClient: Sync + Send {
    async fn poll_workflow_task(
        &self,
        task_queue: TaskQueue,
    ) -> Result<PollWorkflowTaskQueueResponse>;
    async fn poll_activity_task(
        &self,
        task_queue: String,
        max_tasks_per_sec: Option<f64>,
    ) -> Result<PollActivityTaskQueueResponse>;
    async fn complete_workflow_task(
        &self,
        request: WorkflowTaskCompletion,
    ) -> Result<RespondWorkflowTaskCompletedResponse>;
    async fn complete_activity_task(
        &self,
        task_token: TaskToken,
        result: Option<Payloads>,
    ) -> Result<RespondActivityTaskCompletedResponse>;
    async fn record_activity_heartbeat(
        &self,
        task_token: TaskToken,
        details: Option<Payloads>,
    ) -> Result<RecordActivityTaskHeartbeatResponse>;
    async fn cancel_activity_task(
        &self,
        task_token: TaskToken,
        details: Option<Payloads>,
    ) -> Result<RespondActivityTaskCanceledResponse>;
    async fn fail_activity_task(
        &self,
        task_token: TaskToken,
        failure: Option<Failure>,
    ) -> Result<RespondActivityTaskFailedResponse>;
    async fn fail_workflow_task(
        &self,
        task_token: TaskToken,
        cause: WorkflowTaskFailedCause,
        failure: Option<Failure>,
    ) -> Result<RespondWorkflowTaskFailedResponse>;
    async fn get_workflow_execution_history(
        &self,
        workflow_id: String,
        run_id: Option<String>,
        page_token: Vec<u8>,
    ) -> Result<GetWorkflowExecutionHistoryResponse>;
    async fn respond_legacy_query(
        &self,
        task_token: TaskToken,
        query_result: QueryResult,
    ) -> Result<RespondQueryTaskCompletedResponse>;

    #[allow(clippy::needless_lifetimes)] // Clippy is wrong here
    fn capabilities<'a>(&'a self) -> Option<&'a get_system_info_response::Capabilities>;
}

#[async_trait::async_trait]
impl WorkerClient for WorkerClientBag {
    async fn poll_workflow_task(
        &self,
        task_queue: TaskQueue,
    ) -> Result<PollWorkflowTaskQueueResponse> {
        let request = PollWorkflowTaskQueueRequest {
            namespace: self.namespace.clone(),
            task_queue: Some(task_queue),
            identity: self.identity.clone(),
            binary_checksum: self.binary_checksum(),
            worker_version_capabilities: self.worker_version_capabilities(),
        };

        Ok(self
            .client
            .clone()
            .poll_workflow_task_queue(request)
            .await?
            .into_inner())
    }

    async fn poll_activity_task(
        &self,
        task_queue: String,
        max_tasks_per_sec: Option<f64>,
    ) -> Result<PollActivityTaskQueueResponse> {
        let request = PollActivityTaskQueueRequest {
            namespace: self.namespace.clone(),
            task_queue: Some(TaskQueue {
                name: task_queue,
                kind: TaskQueueKind::Normal as i32,
                normal_name: "".to_string(),
            }),
            identity: self.identity.clone(),
            task_queue_metadata: max_tasks_per_sec.map(|tps| TaskQueueMetadata {
                max_tasks_per_second: Some(tps),
            }),
            worker_version_capabilities: self.worker_version_capabilities(),
        };

        Ok(self
            .client
            .clone()
            .poll_activity_task_queue(request)
            .await?
            .into_inner())
    }

    async fn complete_workflow_task(
        &self,
        request: WorkflowTaskCompletion,
    ) -> Result<RespondWorkflowTaskCompletedResponse> {
        let request = RespondWorkflowTaskCompletedRequest {
            task_token: request.task_token.into(),
            commands: request.commands,
            identity: self.identity.clone(),
            sticky_attributes: request.sticky_attributes,
            return_new_workflow_task: request.return_new_workflow_task,
            force_create_new_workflow_task: request.force_create_new_workflow_task,
            worker_version_stamp: self.worker_version_stamp(),
            messages: vec![],
            binary_checksum: self.binary_checksum(),
            query_results: request
                .query_responses
                .into_iter()
                .map(|qr| {
                    let (id, completed_type, query_result, error_message) = qr.into_components();
                    (
                        id,
                        WorkflowQueryResult {
                            result_type: completed_type as i32,
                            answer: query_result,
                            error_message,
                        },
                    )
                })
                .collect(),
            namespace: self.namespace.clone(),
            sdk_metadata: Some(request.sdk_metadata),
            metering_metadata: Some(request.metering_metadata),
        };
        Ok(self
            .client
            .clone()
            .respond_workflow_task_completed(request)
            .await?
            .into_inner())
    }

    async fn complete_activity_task(
        &self,
        task_token: TaskToken,
        result: Option<Payloads>,
    ) -> Result<RespondActivityTaskCompletedResponse> {
        Ok(self
            .client
            .clone()
            .respond_activity_task_completed(RespondActivityTaskCompletedRequest {
                task_token: task_token.0,
                result,
                identity: self.identity.clone(),
                namespace: self.namespace.clone(),
                worker_version: self.worker_version_stamp(),
            })
            .await?
            .into_inner())
    }

    async fn record_activity_heartbeat(
        &self,
        task_token: TaskToken,
        details: Option<Payloads>,
    ) -> Result<RecordActivityTaskHeartbeatResponse> {
        Ok(self
            .client
            .clone()
            .record_activity_task_heartbeat(RecordActivityTaskHeartbeatRequest {
                task_token: task_token.0,
                details,
                identity: self.identity.clone(),
                namespace: self.namespace.clone(),
            })
            .await?
            .into_inner())
    }

    async fn cancel_activity_task(
        &self,
        task_token: TaskToken,
        details: Option<Payloads>,
    ) -> Result<RespondActivityTaskCanceledResponse> {
        Ok(self
            .client
            .clone()
            .respond_activity_task_canceled(RespondActivityTaskCanceledRequest {
                task_token: task_token.0,
                details,
                identity: self.identity.clone(),
                namespace: self.namespace.clone(),
                worker_version: self.worker_version_stamp(),
            })
            .await?
            .into_inner())
    }

    async fn fail_activity_task(
        &self,
        task_token: TaskToken,
        failure: Option<Failure>,
    ) -> Result<RespondActivityTaskFailedResponse> {
        Ok(self
            .client
            .clone()
            .respond_activity_task_failed(RespondActivityTaskFailedRequest {
                task_token: task_token.0,
                failure,
                identity: self.identity.clone(),
                namespace: self.namespace.clone(),
                // TODO: Implement - https://github.com/temporalio/sdk-core/issues/293
                last_heartbeat_details: None,
                worker_version: self.worker_version_stamp(),
            })
            .await?
            .into_inner())
    }

    async fn fail_workflow_task(
        &self,
        task_token: TaskToken,
        cause: WorkflowTaskFailedCause,
        failure: Option<Failure>,
    ) -> Result<RespondWorkflowTaskFailedResponse> {
        let request = RespondWorkflowTaskFailedRequest {
            task_token: task_token.0,
            cause: cause as i32,
            failure,
            identity: self.identity.clone(),
            binary_checksum: self.binary_checksum(),
            namespace: self.namespace.clone(),
            messages: vec![],
            worker_version: self.worker_version_stamp(),
        };
        Ok(self
            .client
            .clone()
            .respond_workflow_task_failed(request)
            .await?
            .into_inner())
    }

    async fn get_workflow_execution_history(
        &self,
        workflow_id: String,
        run_id: Option<String>,
        page_token: Vec<u8>,
    ) -> Result<GetWorkflowExecutionHistoryResponse> {
        Ok(self
            .client
            .clone()
            .get_workflow_execution_history(GetWorkflowExecutionHistoryRequest {
                namespace: self.namespace.clone(),
                execution: Some(WorkflowExecution {
                    workflow_id,
                    run_id: run_id.unwrap_or_default(),
                }),
                next_page_token: page_token,
                ..Default::default()
            })
            .await?
            .into_inner())
    }

    async fn respond_legacy_query(
        &self,
        task_token: TaskToken,
        query_result: QueryResult,
    ) -> Result<RespondQueryTaskCompletedResponse> {
        let (_, completed_type, query_result, error_message) = query_result.into_components();
        Ok(self
            .client
            .clone()
            .respond_query_task_completed(RespondQueryTaskCompletedRequest {
                task_token: task_token.into(),
                completed_type: completed_type as i32,
                query_result,
                error_message,
                namespace: self.namespace.clone(),
            })
            .await?
            .into_inner())
    }

    fn capabilities(&self) -> Option<&Capabilities> {
        self.client.get_client().inner().capabilities()
    }
}

/// A version of [RespondWorkflowTaskCompletedRequest] that will finish being filled out by the
/// server client
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WorkflowTaskCompletion {
    /// The task token that would've been received from polling for a workflow activation
    pub task_token: TaskToken,
    /// A list of new commands to send to the server, such as starting a timer.
    pub commands: Vec<Command>,
    /// If set, indicate that next task should be queued on sticky queue with given attributes.
    pub sticky_attributes: Option<StickyExecutionAttributes>,
    /// Responses to queries in the `queries` field of the workflow task.
    pub query_responses: Vec<QueryResult>,
    /// Indicate that the task completion should return a new WFT if one is available
    pub return_new_workflow_task: bool,
    /// Force a new WFT to be created after this completion
    pub force_create_new_workflow_task: bool,
    /// SDK-specific metadata to send
    pub sdk_metadata: WorkflowTaskCompletedMetadata,
    /// Metering info
    pub metering_metadata: MeteringMetadata,
}
