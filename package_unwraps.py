#!/usr/bin/env python3
"""Split target/unwrap_sites.tsv into review packages under target/unwrap_pkgs/."""
import os
from collections import defaultdict

os.makedirs("target/unwrap_pkgs", exist_ok=True)

# package name -> list of (file, max_line or None).  (file, N) splits by line.
GROUPS = {
 "pkg01_runtime_store_a": [("modules/flowable-engine/src/persistence/runtime_store.rs", ("lo",))],
 "pkg02_runtime_store_b": [("modules/flowable-engine/src/persistence/runtime_store.rs", ("hi",))],
 "pkg03_historical_migration": [("modules/flowable-engine/src/engine/historical_migration.rs", None)],
 "pkg04_converters": [
    ("modules/flowable-bpmn-converter/src/lib.rs", None),
    ("modules/flowable-bpmn-converter/src/writer.rs", None),
    ("modules/flowable-dmn-converter/src/lib.rs", None),
    ("modules/flowable-dmn-converter/src/writer.rs", None),
    ("modules/flowable-cmmn-converter/src/lib.rs", None),
 ],
 "pkg05_deployment": [
    ("modules/flowable-engine/src/engine/deployment_manager.rs", None),
    ("modules/flowable-engine/src/engine/bpmn_model_cache.rs", None),
    ("modules/flowable-engine/src/engine/deployer/bpmn_deployer.rs", None),
    ("modules/flowable-engine/src/repository/deployment_resource.rs", None),
 ],
 "pkg06_dmn_engine": [("modules/flowable-dmn-engine/src/", None)],
 "pkg07_runtime_service": [
    ("modules/flowable-engine/src/engine/runtime_service.rs", None),
    ("modules/flowable-engine/src/engine/variable_service.rs", None),
    ("modules/flowable-engine/src/engine/data_object_service.rs", None),
 ],
 "pkg08_scripting_service_task": [
    ("modules/flowable-engine/src/bpmn/behavior/service_task_activity_behavior.rs", None),
    ("modules/flowable-engine/src/scripting/", None),
    ("modules/flowable-engine/src/bpmn/http_task.rs", None),
    ("modules/flowable-engine/src/bpmn/http_handler.rs", None),
 ],
 "pkg09_cmmn_engine": [("modules/flowable-cmmn-engine/src/", None)],
 "pkg10_rest_pi_cmmn": [
    ("modules/flowable-rest/src/routes/cmmn.rs", None),
    ("modules/flowable-rest/src/routes/process_instances.rs", None),
    ("modules/flowable-rest/src/routes/process_instances_query.rs", None),
 ],
 "pkg11_rest_tasks_mgmt_hist": [
    ("modules/flowable-rest/src/routes/management.rs", None),
    ("modules/flowable-rest/src/routes/history.rs", None),
    ("modules/flowable-rest/src/routes/tasks.rs", None),
    ("modules/flowable-rest/src/routes/task_variables.rs", None),
 ],
 "pkg12_rest_core": [
    ("modules/flowable-rest/src/lib.rs", None),
    ("modules/flowable-rest/src/routes/idm.rs", None),
    ("modules/flowable-rest/src/security.rs", None),
    ("modules/flowable-rest/src/config.rs", None),
    ("modules/flowable-rest/src/variable_types.rs", None),
 ],
 "pkg13_rest_misc": [
    ("modules/flowable-rest/src/routes/forms.rs", None),
    ("modules/flowable-rest/src/routes/deployments.rs", None),
    ("modules/flowable-rest/src/routes/process_definitions.rs", None),
    ("modules/flowable-rest/src/routes/external_worker.rs", None),
    ("modules/flowable-rest/src/routes/event_registry.rs", None),
    ("modules/flowable-rest/src/routes/metrics.rs", None),
    ("modules/flowable-rest/src/routes/event_subscriptions.rs", None),
    ("modules/flowable-rest/src/routes/models.rs", None),
    ("modules/flowable-rest/src/routes/identity_links.rs", None),
    ("modules/flowable-rest/src/routes/batches.rs", None),
    ("modules/flowable-rest/src/routes/apps.rs", None),
    ("modules/flowable-rest/src/routes/adhoc.rs", None),
    ("modules/flowable-rest/src/routes/signals.rs", None),
    ("modules/flowable-rest/src/routes/rendering.rs", None),
    ("modules/flowable-rest/src/routes/messages.rs", None),
    ("modules/flowable-rest/src/routes/dmn.rs", None),
    ("modules/flowable-rest/src/routes/content.rs", None),
    ("modules/flowable-rest/src/routes/attachments.rs", None),
 ],
 "pkg14_ui_rest": [("modules/flowable-ui-rest/src/", None)],
 "pkg15_form_service": [("modules/flowable-form-service/src/", None)],
 "pkg16_http_event_registry": [
    ("modules/flowable-http-service/src/", None),
    ("modules/flowable-event-registry-service/src/", None),
 ],
 "pkg17_content_and_small_services": [
    ("modules/flowable-content-service/src/", None),
    ("modules/flowable-history-service/src/", None),
    ("modules/flowable-mail-service/src/", None),
    ("modules/flowable-modeler-service/src/lib.rs", None),
    ("modules/flowable-modeler-protocol/src/", None),
    ("modules/flowable-image-generator/src/", None),
    ("modules/flowable-dmn-image-generator/src/", None),
    ("modules/flowable-cmmn-image-generator/src/", None),
    ("modules/flowable-bpmn-layout/src/", None),
    ("modules/flowable-cmmn-model/src/", None),
 ],
 "pkg18_identity_jwks_config": [
    ("modules/flowable-engine/src/engine/identity_service.rs", None),
    ("modules/flowable-engine/src/engine/async_executor.rs", None),
    ("modules/flowable-engine/src/engine/process_engine.rs", None),
    ("modules/flowable-engine/src/service/jwks.rs", None),
    ("modules/flowable-engine/src/service/config.rs", None),
 ],
 "pkg19_timers_async": [
    ("modules/flowable-engine/src/service/timer_coordination_service.rs", None),
    ("modules/flowable-engine/src/service/revocation.rs", None),
    ("modules/flowable-engine/src/service/external_auth.rs", None),
    ("modules/flowable-engine/src/engine/timer_executor.rs", None),
    ("modules/flowable-engine/src/engine/batch_service.rs", None),
    ("modules/flowable-engine/src/engine/activation_coordinator.rs", None),
    ("modules/flowable-engine/src/engine/time_source.rs", None),
    ("modules/flowable-engine/src/engine/async_job_acquisition.rs", None),
    ("modules/flowable-engine/src/engine/timer_job_acquisition.rs", None),
    ("modules/flowable-engine/src/engine/reset_expired_jobs.rs", None),
    ("modules/flowable-engine/src/engine/async_history_executor.rs", None),
    ("modules/flowable-engine/src/engine/history_job_dispatcher.rs", None),
    ("modules/flowable-engine/src/service/issuer_health.rs", None),
    ("modules/flowable-engine/src/service/rate_limit.rs", None),
    ("modules/flowable-engine/src/engine/timer_worker.rs", None),
    ("modules/flowable-engine/src/service/identity_sync.rs", None),
    ("modules/flowable-engine/src/service/auth.rs", None),
 ],
 "pkg20_mgmt_history_services": [
    ("modules/flowable-engine/src/engine/management_service.rs", None),
    ("modules/flowable-engine/src/engine/task_service.rs", None),
    ("modules/flowable-engine/src/engine/repository_service.rs", None),
    ("modules/flowable-engine/src/engine/history_service.rs", None),
    ("modules/flowable-engine/src/history/", None),
    ("modules/flowable-engine/src/engine/identity_link_service.rs", None),
    ("modules/flowable-engine/src/engine/entity_link_service.rs", None),
    ("modules/flowable-engine/src/engine/external_worker_service.rs", None),
    ("modules/flowable-engine/src/engine/rest_service.rs", None),
    ("modules/flowable-engine/src/engine/history_cleaning.rs", None),
    ("modules/flowable-engine/src/runtime/execution.rs", None),
 ],
 "pkg21_agenda_behavior_a": [
    ("modules/flowable-engine/src/agenda/", None),
    ("modules/flowable-engine/src/bpmn/behavior/multi_instance_support.rs", None),
    ("modules/flowable-engine/src/bpmn/behavior/end_event_activity_behavior.rs", None),
    ("modules/flowable-engine/src/bpmn/behavior/call_activity_behavior.rs", None),
    ("modules/flowable-engine/src/bpmn/behavior/receive_task_activity_behavior.rs", None),
    ("modules/flowable-engine/src/bpmn/behavior/user_task_activity_behavior.rs", None),
    ("modules/flowable-engine/src/bpmn/behavior/sub_process_activity_behavior.rs", None),
    ("modules/flowable-engine/src/bpmn/behavior/intermediate_catch_event_activity_behavior.rs", None),
    ("modules/flowable-engine/src/bpmn/behavior/case_task_activity_behavior.rs", None),
 ],
 "pkg22_behavior_b_validation": [
    ("modules/flowable-engine/src/bpmn/behavior/", None),
    ("modules/flowable-engine/src/bpmn/fault.rs", None),
    ("modules/flowable-engine/src/bpmn/skip_expression.rs", None),
    ("modules/flowable-engine/src/bpmn/timer_util.rs", None),
    ("modules/flowable-engine/src/bpmn/event_registry_correlation.rs", None),
    ("modules/flowable-engine/src/bpmn/execution_graph_util.rs", None),
    ("modules/flowable-engine/src/bpmn/parser/", None),
    ("modules/flowable-engine/src/bpmn/listener/", None),
    ("modules/flowable-engine/src/validation/", None),
 ],
 "pkg23_cmds": [
    ("modules/flowable-engine/src/cmd/", None),
    ("modules/flowable-engine/src/interceptor/", None),
 ],
 "pkg24_persistence": [
    ("modules/flowable-persistence/src/", None),
    ("modules/flowable-engine/src/persistence/db_session.rs", None),
    ("modules/flowable-engine/src/persistence/db_store.rs", None),
    ("modules/flowable-engine/src/persistence/entity_mapping.rs", None),
 ],
 "pkg25_bins_bootstrap_app": [
    ("modules/flowable-engine/src/bin/", None),
    ("modules/flowable-modeler-service/src/bin/", None),
    ("modules/flowable-platform-bootstrap/src/", None),
    ("modules/flowable-app-engine/src/", None),
 ],
 "pkg26_engine_common_el": [("modules/flowable-engine-common/src/", None)],
}

def load_sites():
    per_file = defaultdict(list)
    for raw in open("target/unwrap_sites.tsv", encoding="utf-8"):
        f, ln, text = raw.rstrip("\n").split("\t", 2)
        per_file[f].append((int(ln), text))
    for v in per_file.values():
        v.sort()
    return per_file

def main():
    per_file = load_sites()
    # median split for runtime_store
    rs = per_file["modules/flowable-engine/src/persistence/runtime_store.rs"]
    mid = rs[len(rs)//2][0]

    assigned = set()
    outdir = "target/unwrap_pkgs"
    summary = []
    for pkg, specs in GROUPS.items():
        rows = []
        for spec in specs:
            path, flag = spec
            if path.endswith("/"):
                files = [f for f in per_file if f.startswith(path)]
            else:
                files = [path] if path in per_file else []
            for f in files:
                for ln, text in per_file[f]:
                    if flag == ("lo",) and ln > mid: continue
                    if flag == ("hi",) and ln <= mid: continue
                    rows.append((f, ln, text))
                    assigned.add((f, ln))
        with open(f"{outdir}/{pkg}.tsv", "w", encoding="utf-8") as o:
            for f, ln, text in sorted(rows):
                o.write(f"{f}\t{ln}\t{text}\n")
        summary.append((pkg, len(rows)))
    # remainder
    rest = []
    for f, lst in per_file.items():
        for ln, text in lst:
            if (f, ln) not in assigned:
                rest.append((f, ln, text))
    with open(f"{outdir}/pkg99_remainder.tsv", "w", encoding="utf-8") as o:
        for f, ln, text in sorted(rest):
            o.write(f"{f}\t{ln}\t{text}\n")
    for pkg, n in summary:
        print(f"{n:5d}  {pkg}")
    print(f"{len(rest):5d}  pkg99_remainder")
    if rest:
        from collections import Counter
        print(Counter(f for f, _, _ in rest))

if __name__ == "__main__":
    main()
