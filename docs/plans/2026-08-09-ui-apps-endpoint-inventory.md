# Stream B endpoint inventory (Java 6.8.0)

Generated from `$J/modules/flowable-ui/flowable-ui-{admin,task}-rest` mapping annotations.
Paths are relative to app context: admin=`/admin-app`, task=`/app`.

## Admin (`flowable-ui-admin-rest`) — ~137

### Server configs / engine info
| Method | Path | Java |
|--------|------|------|
| GET | `/rest/server-configs` | ServerConfigsResource.getServers |
| GET | `/rest/server-configs/default/{endpointTypeCode}` | ServerConfigsResource.getDefaultServerConfig |
| PUT | `/rest/server-configs/{serverId}` | ServerConfigsResource.updateServer |
| GET | `/rest/admin/engine-info/{endpointTypeCode}` | ProcessEngineInfoClientResource |

### Deployments / models (PROCESS)
| Method | Path |
|--------|------|
| GET/POST | `/rest/admin/deployments` |
| GET/DELETE | `/rest/admin/deployments/{deploymentId}` |
| GET | `/rest/admin/models` |

### Process definitions / instances / tasks / jobs
| Method | Path |
|--------|------|
| GET | `/rest/admin/process-definitions` |
| GET/PUT | `/rest/admin/process-definitions/{definitionId}` |
| GET | `/rest/admin/process-definitions/{definitionId}/process-instances` |
| GET | `/rest/admin/process-definitions/{definitionId}/jobs` |
| POST | `/rest/admin/process-definitions/{definitionId}/batch-migrate` |
| GET | `/rest/admin/process-definitions/{processDefinitionId}/model-json` |
| POST | `/rest/admin/process-instances` |
| GET/POST | `/rest/admin/process-instances/{processInstanceId}` |
| GET | `.../tasks`, `.../variables`, `.../subprocesses`, `.../jobs`, `.../decision-executions` |
| PUT/POST/DELETE | `.../variables`, `.../variables/{variableName}` |
| POST | `.../change-state`, `.../migrate` |
| GET | `.../model-json`, `.../history-model-json` |
| POST | `/rest/admin/tasks` |
| GET/DELETE/POST/PUT | `/rest/admin/tasks/{taskId}` |
| GET | `.../subtasks`, `.../variables`, `.../identitylinks` |
| GET | `/rest/admin/jobs` |
| GET/DELETE/POST | `/rest/admin/jobs/{jobId}` |
| GET | `/rest/admin/jobs/{jobId}/exception-stacktrace` |
| GET | `/rest/admin/event-subscriptions` |
| GET/POST | `/rest/admin/event-subscriptions/{eventSubscriptionId}` |
| GET | `/rest/admin/batches` |
| GET/DELETE | `/rest/admin/batches/{batchId}` |
| GET | `.../batch-parts`, `.../batch-document` |
| GET | `/rest/admin/batch-parts/{batchPartId}` (+ document) |

### CMMN / DMN / FORM / APP / CONTENT
| Domain | Path prefix |
|--------|-------------|
| CMMN | `/rest/admin/cmmn-deployments`, `case-definitions`, `case-instances`, `cmmn-tasks`, `cmmn-jobs` |
| DMN | `/rest/admin/decision-table-deployments`, `decision-tables`, history/audit |
| FORM | `/rest/admin/form-deployments`, `form-definitions`, `form-instances` |
| APP | `/rest/admin/app-deployments`, `app-definitions` |
| CONTENT | `/rest/admin/content-items` |

Display JSON: `/rest/admin/process-definitions|process-instances|case-definitions|case-instances/.../model-json`.

## Task (`flowable-ui-task-rest`) — ~109

| Area | Paths |
|------|-------|
| Tasks | `POST /rest/tasks`, `GET/PUT /rest/tasks/{id}`, subtasks, actions (complete/assign/involve/claim) |
| Query | `POST /rest/query/tasks`, `POST /rest/query/history/tasks` |
| Forms | `GET/POST /rest/task-forms/{taskId}`, `.../save-form` |
| Comments | `GET/POST /rest/tasks/{id}/comments`, process-instance comments |
| Content | `/rest/tasks|process-instances|case-instances|content/**` |
| Process | definitions, instances, query, start-form, model-json, delete |
| Case | definitions, instances, stages, milestones, plan-items, model-json |
| IDM helpers | `/rest/workflow-users`, `/rest/workflow-groups`, `/rest/users/{id}` |
| Apps | `/rest/runtime/app-definitions` |
| Debugger | `/rest/debugger/**` |

## Stream B implementation status (this branch)

- [x] B0 scaffold + health probes
- [x] Admin ServerConfig CRUD + AES encrypt + **JSON file persistence**
- [x] Admin proxy core + primary domains (process/cmmn/dmn/form/app/content)
- [x] Admin multipart deploy (process/cmmn/dmn/form/app)
- [x] Admin display-json (BpmnModel DI → elements/flows/highlight)
- [x] Task RestVariable converters
- [x] Task aggregation: create/query/action/form/comments/process/users/groups
- [x] Case list/start/get/delete/query (via ProcessEngine.cmmn_engine)
- [x] Related content list/create/get/delete
- [x] Debugger gate + breakpoints + executions/variables (env-flagged)
- [x] Mounted into `flowable-rest` via stream A `ui_router()`
