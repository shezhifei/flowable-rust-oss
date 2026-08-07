package org.flowable.rust.contract;

import java.io.IOException;
import java.io.InputStream;
import java.net.InetSocketAddress;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Collection;
import java.util.Collections;
import java.util.Comparator;
import java.util.Date;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Objects;
import java.util.Set;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicReference;
import java.util.function.BooleanSupplier;
import java.util.function.Supplier;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ArrayNode;
import com.fasterxml.jackson.databind.node.ObjectNode;
import com.sun.net.httpserver.HttpExchange;
import com.sun.net.httpserver.HttpServer;
import org.flowable.common.engine.api.FlowableException;
import org.flowable.common.engine.impl.cfg.multitenant.TenantInfoHolder;
import org.flowable.common.engine.impl.util.DefaultClockImpl;
import org.flowable.engine.IdentityService;
import org.flowable.engine.ProcessEngine;
import org.flowable.engine.ProcessEngineConfiguration;
import org.flowable.engine.ManagementService;
import org.flowable.engine.RepositoryService;
import org.flowable.engine.RuntimeService;
import org.flowable.engine.TaskService;
import org.flowable.engine.impl.cfg.ProcessEngineConfigurationImpl;
import org.flowable.engine.repository.Deployment;
import org.flowable.engine.runtime.ProcessInstance;
import org.flowable.idm.api.Group;
import org.flowable.idm.api.User;
import org.flowable.job.api.Job;
import org.flowable.job.api.FlowableUnrecoverableJobException;
import org.flowable.job.service.impl.asyncexecutor.AsyncJobExecutorConfiguration;
import org.flowable.job.service.impl.asyncexecutor.DefaultAsyncJobExecutor;
import org.flowable.job.service.impl.asyncexecutor.multitenant.SharedExecutorServiceAsyncExecutor;
import org.flowable.job.service.impl.cmd.AcquireJobsCmd;
import org.flowable.job.service.impl.persistence.entity.JobInfoEntity;
import org.flowable.task.api.Task;
import org.flowable.task.api.TaskQuery;

public final class JavaHttpContractRunner {

    private static final ObjectMapper OBJECT_MAPPER = new ObjectMapper();
    private static long FIXED_CLOCK_MILLIS = 1_700_000_000_000L;
    private static long ASYNC_RETRY_ADVANCE_MILLIS = 10_001L;
    private static final long WALL_TIMEOUT_MILLIS = 15_000L;
    private static String AUTOMATIC_EXECUTOR_LOCK_OWNER = "java-http-differential";
    private static String UNLOCK_OWNED_JOBS_LOCK_OWNER = "unlock-owned-jobs-differential";
    private static String SHARED_UNLOCK_OTHER_OWNER = "shared-unlock-other-owner";
    private static String SHARED_TENANT_A = "tenant-a";
    private static String SHARED_TENANT_B = "tenant-b";
    private static String SHARED_TENANT_C = "tenant-c";
    private static List<String> SHARED_REGISTERED_TENANTS = List.of(
            SHARED_TENANT_A,
            SHARED_TENANT_B);
    private static List<String> DEFAULT_OBSERVE_VARIABLES = List.of(
            "contractRequestMethod",
            "contractRequestBody",
            "contractDisallowRedirects",
            "contractResponseStatusCode",
            "contractErrorMessage",
            "responseBody");

    private JavaHttpContractRunner() {
    }

    public static void main(String[] args) throws Exception {
        if (args.length != 2) {
            throw new IllegalArgumentException("Expected <fixture-directory> <output-json>");
        }

        Path fixtureDirectory = Path.of(args[0]).toAbsolutePath().normalize();
        Path output = Path.of(args[1]).toAbsolutePath().normalize();
        JsonNode fixture = OBJECT_MAPPER.readTree(fixtureDirectory.resolve("cases.json").toFile());
        applyFixtureConfig(fixture);

        DefaultClockImpl sharedClock = new DefaultClockImpl();
        sharedClock.setCurrentTime(new Date(FIXED_CLOCK_MILLIS));
        ProcessEngine processEngine = ProcessEngineConfiguration
                .createStandaloneInMemProcessEngineConfiguration()
                .setDatabaseSchemaUpdate(ProcessEngineConfiguration.DB_SCHEMA_UPDATE_TRUE)
                .setAsyncExecutorActivate(false)
                .setClock(sharedClock)
                .buildProcessEngine();

        ObjectNode root = OBJECT_MAPPER.createObjectNode();
        root.put("engine", "flowable-java");
        root.put("flowableVersion", fixture.path("flowableJavaVersion").asText());
        ObjectNode cases = root.putObject("cases");

        try {
            for (JsonNode contractCase : fixture.path("cases")) {
                String id = contractCase.path("id").asText();
                if (contractCase.has("operations")
                        && contractCase.get("operations").isArray()
                        && contractCase.get("operations").size() > 0) {
                    cases.set(id, runOperationsCase(processEngine, fixtureDirectory, contractCase));
                } else if ("automaticAsyncRetry".equals(contractCase.path("execution").asText())) {
                    cases.set(id, runAutomaticAsyncRetryCase(fixtureDirectory, contractCase));
                } else if ("unlockOwnedJobs".equals(contractCase.path("execution").asText())) {
                    cases.set(id, runUnlockOwnedJobsCase(fixtureDirectory, contractCase));
                } else if ("sharedMultiTenantUnlockOwnedJobs".equals(
                        contractCase.path("execution").asText())) {
                    cases.set(id, runSharedMultiTenantUnlockOwnedJobsCase(
                            fixtureDirectory,
                            contractCase));
                } else {
                    cases.set(id, runCase(processEngine, fixtureDirectory, contractCase));
                }
            }
        } finally {
            processEngine.close();
        }

        Files.createDirectories(output.getParent());
        OBJECT_MAPPER.writerWithDefaultPrettyPrinter().writeValue(output.toFile(), root);
    }

    private static void applyFixtureConfig(JsonNode fixture) {
        if (fixture.hasNonNull("fixedClockMillis")) {
            FIXED_CLOCK_MILLIS = fixture.path("fixedClockMillis").asLong();
        }
        if (fixture.hasNonNull("asyncRetryAdvanceMillis")) {
            ASYNC_RETRY_ADVANCE_MILLIS = fixture.path("asyncRetryAdvanceMillis").asLong();
        }
        if (fixture.hasNonNull("automaticLockOwner")) {
            AUTOMATIC_EXECUTOR_LOCK_OWNER = fixture.path("automaticLockOwner").asText();
        }
        if (fixture.hasNonNull("unlockOwnedJobsLockOwner")) {
            UNLOCK_OWNED_JOBS_LOCK_OWNER = fixture.path("unlockOwnedJobsLockOwner").asText();
        }
        if (fixture.hasNonNull("sharedUnlockOtherOwner")) {
            SHARED_UNLOCK_OTHER_OWNER = fixture.path("sharedUnlockOtherOwner").asText();
        }
        if (fixture.hasNonNull("sharedTenantA")) {
            SHARED_TENANT_A = fixture.path("sharedTenantA").asText();
        }
        if (fixture.hasNonNull("sharedTenantB")) {
            SHARED_TENANT_B = fixture.path("sharedTenantB").asText();
        }
        if (fixture.hasNonNull("sharedTenantC")) {
            SHARED_TENANT_C = fixture.path("sharedTenantC").asText();
        }
        SHARED_REGISTERED_TENANTS = List.of(SHARED_TENANT_A, SHARED_TENANT_B);
        if (fixture.has("observeVariables") && fixture.get("observeVariables").isArray()) {
            List<String> names = new ArrayList<>();
            fixture.get("observeVariables").forEach(node -> names.add(node.asText()));
            DEFAULT_OBSERVE_VARIABLES = List.copyOf(names);
        }
    }

    private static List<String> resolveObserveVariables(JsonNode contractCase) {
        if (contractCase.has("observeVariables") && contractCase.get("observeVariables").isArray()) {
            List<String> names = new ArrayList<>();
            contractCase.get("observeVariables").forEach(node -> names.add(node.asText()));
            return names;
        }
        return DEFAULT_OBSERVE_VARIABLES;
    }

    private static List<String> resolveObserveFields(JsonNode contractCase) {
        if (contractCase.has("observe") && contractCase.get("observe").isArray()) {
            List<String> fields = new ArrayList<>();
            contractCase.get("observe").forEach(node -> fields.add(node.asText()));
            return fields;
        }
        return List.of("tasks", "variables", "processEnded", "error");
    }

    /**
     * Generic scripted case: case declares deploy/start/completeTask/trigger/signal/
     * setVariable/snapshot (and optional httpStub). No hard-coded execution mode.
     */
    private static ObjectNode runOperationsCase(
            ProcessEngine processEngine,
            Path fixtureDirectory,
            JsonNode contractCase) throws Exception {

        RuntimeService runtimeService = processEngine.getRuntimeService();
        TaskService taskService = processEngine.getTaskService();
        RepositoryService repositoryService = processEngine.getRepositoryService();
        ManagementService managementService = processEngine.getManagementService();
        IdentityService identityService = processEngine.getIdentityService();
        ProcessEngineConfiguration engineConfig = processEngine.getProcessEngineConfiguration();

        // Reset logical clock at the start of every operations case.
        engineConfig.getClock().setCurrentTime(new Date(FIXED_CLOCK_MILLIS));

        List<String> observeVariables = resolveObserveVariables(contractCase);
        List<String> observeFields = resolveObserveFields(contractCase);
        String processDefinitionId = null;
        String processInstanceId = null;
        String error = null;
        List<String> taskQueryResult = new ArrayList<>();
        HttpServer server = null;
        String endpoint = null;
        List<CapturedRequest> capturedRequests = Collections.synchronizedList(new ArrayList<>());
        AtomicInteger responseAttempt = new AtomicInteger();

        try {
            for (JsonNode operation : contractCase.path("operations")) {
                String op = operation.path("op").asText();
                switch (op) {
                    case "createUser" -> {
                        String userId = operation.path("userId").asText();
                        User user = identityService.newUser(userId);
                        identityService.saveUser(user);
                    }
                    case "createGroup" -> {
                        String groupId = operation.path("groupId").asText();
                        Group group = identityService.newGroup(groupId);
                        if (operation.hasNonNull("groupName")) {
                            group.setName(operation.path("groupName").asText());
                        } else {
                            group.setName(groupId);
                        }
                        identityService.saveGroup(group);
                    }
                    case "createMembership" -> identityService.createMembership(
                            operation.path("userId").asText(),
                            operation.path("groupId").asText());
                    case "queryTasks" -> {
                        TaskQuery query = taskService.createTaskQuery();
                        if (processInstanceId != null) {
                            query = query.processInstanceId(processInstanceId);
                        }
                        query = applyJavaTaskFilters(query, operation);
                        if (operation.has("or") && operation.get("or").isArray()
                                && operation.get("or").size() > 0) {
                            query = query.or();
                            for (JsonNode term : operation.get("or")) {
                                query = applyJavaTaskFilters(query, term);
                            }
                            query = query.endOr();
                        }
                        List<Task> hits = query.list();
                        List<String> keys = new ArrayList<>();
                        for (Task task : hits) {
                            keys.add(task.getTaskDefinitionKey());
                        }
                        Collections.sort(keys);
                        taskQueryResult = keys;
                    }
                    case "advanceClock" -> {
                        long millis = operation.path("millis").asLong();
                        Date current = engineConfig.getClock().getCurrentTime();
                        engineConfig.getClock().setCurrentTime(
                                new Date(current.getTime() + millis));
                    }
                    case "executeDueTimers" -> {
                        Date now = engineConfig.getClock().getCurrentTime();
                        List<Job> timers = processInstanceId == null
                                ? managementService.createTimerJobQuery().list()
                                : managementService.createTimerJobQuery()
                                        .processInstanceId(processInstanceId)
                                        .list();
                        for (Job timer : timers) {
                            if (timer.getDuedate() != null
                                    && !timer.getDuedate().after(now)) {
                                try {
                                    Job executable = managementService
                                            .moveTimerToExecutableJob(timer.getId());
                                    managementService.executeJob(executable.getId());
                                } catch (RuntimeException ignored) {
                                    // Attempt semantics: failures surface via snapshot.
                                }
                            }
                        }
                    }
                    case "httpStub" -> {
                        String expectedPath = operation.path("path").asText();
                        if (server != null) {
                            server.stop(0);
                        }
                        server = HttpServer.create(new InetSocketAddress("127.0.0.1", 0), 0);
                        JsonNode httpCase = operation;
                        server.createContext(expectedPath, exchange -> handleRequest(
                                exchange,
                                httpCaseAsContract(httpCase),
                                capturedRequests,
                                responseAttempt.getAndIncrement()));
                        server.start();
                        endpoint = "http://127.0.0.1:" + server.getAddress().getPort() + expectedPath;
                    }
                    case "deploy" -> {
                        String bpmnName = operation.hasNonNull("bpmn")
                                ? operation.path("bpmn").asText()
                                : contractCase.path("bpmn").asText();
                        String bpmnXml = Files.readString(
                                fixtureDirectory.resolve(bpmnName),
                                StandardCharsets.UTF_8);
                        try {
                            var deploymentBuilder = repositoryService.createDeployment()
                                    .name("contract-" + contractCase.path("id").asText())
                                    .addString(bpmnName, bpmnXml);
                            if (operation.hasNonNull("tenantId")) {
                                deploymentBuilder.tenantId(operation.path("tenantId").asText());
                            }
                            Deployment deployment = deploymentBuilder.deploy();
                            List<org.flowable.engine.repository.ProcessDefinition> definitions =
                                    repositoryService.createProcessDefinitionQuery()
                                            .deploymentId(deployment.getId())
                                            .list();
                            // Multi-process deployments (e.g. call activity parent+child)
                            // require processDefinitionKey on start; leave id null when
                            // more than one definition was added by this deploy.
                            processDefinitionId = definitions.size() == 1
                                    ? definitions.get(0).getId()
                                    : null;
                        } catch (RuntimeException exception) {
                            error = deepestMessage(exception);
                        }
                    }
                    case "start" -> {
                        Map<String, Object> variables = new java.util.LinkedHashMap<>();
                        if (operation.has("variables") && operation.get("variables").isObject()) {
                            operation.get("variables").fields().forEachRemaining(entry ->
                                    variables.put(entry.getKey(), jsonNodeToJava(entry.getValue())));
                        }
                        if (endpoint != null) {
                            variables.put("endpoint", endpoint);
                        }
                        try {
                            var builder = runtimeService.createProcessInstanceBuilder();
                            if (operation.hasNonNull("processDefinitionKey")) {
                                builder.processDefinitionKey(
                                        operation.path("processDefinitionKey").asText());
                            } else {
                                if (processDefinitionId == null) {
                                    throw new IllegalStateException(
                                            "start requires a prior deploy or processDefinitionKey");
                                }
                                builder.processDefinitionId(processDefinitionId);
                            }
                            if (operation.hasNonNull("businessKey")) {
                                builder.businessKey(operation.path("businessKey").asText());
                            }
                            if (operation.hasNonNull("tenantId")) {
                                builder.tenantId(operation.path("tenantId").asText());
                            }
                            if (!variables.isEmpty()) {
                                builder.variables(variables);
                            }
                            ProcessInstance processInstance = builder.start();
                            processInstanceId = processInstance.getId();
                        } catch (RuntimeException exception) {
                            error = deepestMessage(exception);
                        }
                    }
                    case "completeTask" -> {
                        String key = operation.path("taskDefinitionKey").asText();
                        Task task = taskService.createTaskQuery()
                                .processInstanceId(processInstanceId)
                                .taskDefinitionKey(key)
                                .singleResult();
                        if (task == null) {
                            throw new IllegalStateException("No active task with key " + key);
                        }
                        if (operation.has("variables") && operation.get("variables").isObject()) {
                            Map<String, Object> variables = new java.util.LinkedHashMap<>();
                            operation.get("variables").fields().forEachRemaining(entry ->
                                    variables.put(entry.getKey(), jsonNodeToJava(entry.getValue())));
                            taskService.complete(task.getId(), variables);
                        } else {
                            taskService.complete(task.getId());
                        }
                    }
                    case "setVariable" -> {
                        String name = operation.path("name").asText();
                        Object value = jsonNodeToJava(operation.get("value"));
                        if (operation.path("local").asBoolean(false)) {
                            runtimeService.setVariableLocal(processInstanceId, name, value);
                        } else {
                            runtimeService.setVariable(processInstanceId, name, value);
                        }
                    }
                    case "setVariableLocal" -> {
                        String name = operation.path("name").asText();
                        Object value = jsonNodeToJava(operation.get("value"));
                        if (operation.hasNonNull("taskDefinitionKey")) {
                            Task task = taskService.createTaskQuery()
                                    .processInstanceId(processInstanceId)
                                    .taskDefinitionKey(operation.path("taskDefinitionKey").asText())
                                    .singleResult();
                            if (task == null) {
                                throw new IllegalStateException(
                                        "No active task with key "
                                                + operation.path("taskDefinitionKey").asText());
                            }
                            taskService.setVariableLocal(task.getId(), name, value);
                        } else {
                            runtimeService.setVariableLocal(processInstanceId, name, value);
                        }
                    }
                    case "trigger" -> {
                        // Trigger waiting intermediate catch executions for the PI.
                        List<org.flowable.engine.runtime.Execution> waiting =
                                runtimeService.createExecutionQuery()
                                        .processInstanceId(processInstanceId)
                                        .activityId(operation.path("activityId").asText(null))
                                        .list();
                        if (waiting.isEmpty()) {
                            waiting = runtimeService.createExecutionQuery()
                                    .processInstanceId(processInstanceId)
                                    .onlyChildExecutions()
                                    .list();
                        }
                        for (org.flowable.engine.runtime.Execution execution : waiting) {
                            if (execution.getActivityId() != null
                                    && !execution.getId().equals(processInstanceId)) {
                                try {
                                    runtimeService.trigger(execution.getId());
                                } catch (RuntimeException ignored) {
                                    // Not every child execution is triggerable.
                                }
                            }
                        }
                    }
                    case "signalEvent" -> {
                        String signalName = operation.path("signalName").asText();
                        // Deliver to every waiting subscription for this PI (boundary + intermediate).
                        List<org.flowable.engine.runtime.Execution> waiting =
                                runtimeService.createExecutionQuery()
                                        .processInstanceId(processInstanceId)
                                        .signalEventSubscriptionName(signalName)
                                        .list();
                        if (waiting.isEmpty()) {
                            runtimeService.signalEventReceived(signalName);
                        } else {
                            for (org.flowable.engine.runtime.Execution execution : waiting) {
                                runtimeService.signalEventReceived(signalName, execution.getId());
                            }
                        }
                    }
                    case "messageEvent" -> {
                        String messageName = operation.path("messageName").asText();
                        List<org.flowable.engine.runtime.Execution> waiting =
                                runtimeService.createExecutionQuery()
                                        .processInstanceId(processInstanceId)
                                        .messageEventSubscriptionName(messageName)
                                        .list();
                        if (waiting.isEmpty()) {
                            throw new IllegalStateException(
                                    "No message subscription for " + messageName);
                        }
                        for (org.flowable.engine.runtime.Execution execution : waiting) {
                            runtimeService.messageEventReceived(messageName, execution.getId());
                        }
                    }
                    case "triggerBoundary" -> {
                        String activityId = operation.path("activityId").asText();
                        List<org.flowable.engine.runtime.Execution> waiting =
                                runtimeService.createExecutionQuery()
                                        .processInstanceId(processInstanceId)
                                        .activityId(activityId)
                                        .list();
                        if (waiting.isEmpty()) {
                            throw new IllegalStateException(
                                    "No execution at boundary activity " + activityId);
                        }
                        // Prefer message then signal delivery via the execution id.
                        for (org.flowable.engine.runtime.Execution execution : waiting) {
                            runtimeService.trigger(execution.getId());
                        }
                    }
                    case "claimTask" -> {
                        Task task = requireTask(
                                taskService,
                                processInstanceId,
                                operation.path("taskDefinitionKey").asText());
                        taskService.claim(task.getId(), operation.path("userId").asText());
                    }
                    case "delegateTask" -> {
                        Task task = requireTask(
                                taskService,
                                processInstanceId,
                                operation.path("taskDefinitionKey").asText());
                        taskService.delegateTask(task.getId(), operation.path("userId").asText());
                    }
                    case "resolveTask" -> {
                        Task task = requireTask(
                                taskService,
                                processInstanceId,
                                operation.path("taskDefinitionKey").asText());
                        taskService.resolveTask(task.getId());
                    }
                    case "executeJobs" -> {
                        List<Job> jobs = managementService.createJobQuery()
                                .processInstanceId(processInstanceId)
                                .list();
                        for (Job job : jobs) {
                            try {
                                managementService.executeJob(job.getId());
                            } catch (RuntimeException ignored) {
                                // Attempt semantics: failures are observable via snapshot.
                            }
                        }
                    }
                    case "snapshot" -> {
                        // Final snapshot is built after the loop.
                    }
                    default -> throw new IllegalArgumentException(
                            "Unsupported differential operation: " + op);
                }
            }

            return buildOperationsSnapshot(
                    runtimeService,
                    taskService,
                    managementService,
                    processInstanceId,
                    error,
                    observeVariables,
                    observeFields,
                    taskQueryResult);
        } finally {
            if (server != null) {
                server.stop(0);
            }
        }
    }

    private static TaskQuery applyJavaTaskFilters(TaskQuery query, JsonNode filters) {
        if (filters.hasNonNull("candidateUser")) {
            query = query.taskCandidateUser(filters.path("candidateUser").asText());
        }
        if (filters.hasNonNull("candidateGroup")) {
            query = query.taskCandidateGroup(filters.path("candidateGroup").asText());
        }
        if (filters.hasNonNull("candidateOrAssigned")) {
            query = query.taskCandidateOrAssigned(filters.path("candidateOrAssigned").asText());
        }
        if (filters.hasNonNull("assignee")) {
            query = query.taskAssignee(filters.path("assignee").asText());
        }
        if (filters.hasNonNull("taskName")) {
            query = query.taskName(filters.path("taskName").asText());
        }
        if (filters.hasNonNull("taskDefinitionKey")) {
            query = query.taskDefinitionKey(filters.path("taskDefinitionKey").asText());
        }
        if (filters.path("ignoreAssignee").asBoolean(false)) {
            query = query.ignoreAssigneeValue();
        }
        return query;
    }

    private static Task requireTask(
            TaskService taskService,
            String processInstanceId,
            String taskDefinitionKey) {
        Task task = taskService.createTaskQuery()
                .processInstanceId(processInstanceId)
                .taskDefinitionKey(taskDefinitionKey)
                .singleResult();
        if (task == null) {
            throw new IllegalStateException("No active task with key " + taskDefinitionKey);
        }
        return task;
    }

    private static JsonNode httpCaseAsContract(JsonNode httpOp) {
        ObjectNode contract = OBJECT_MAPPER.createObjectNode();
        contract.set("responseStatus", httpOp.path("responseStatus"));
        contract.set("responseBody", httpOp.path("responseBody"));
        if (httpOp.has("subsequentResponses")) {
            contract.set("subsequentResponses", httpOp.get("subsequentResponses"));
        }
        return contract;
    }

    private static Object jsonNodeToJava(JsonNode node) {
        if (node == null || node.isNull()) {
            return null;
        }
        if (node.isBoolean()) {
            return node.asBoolean();
        }
        if (node.isInt()) {
            return node.asInt();
        }
        if (node.isLong()) {
            return node.asLong();
        }
        if (node.isNumber()) {
            return node.numberValue();
        }
        if (node.isTextual()) {
            return node.asText();
        }
        if (node.isArray() || node.isObject()) {
            return OBJECT_MAPPER.convertValue(node, Object.class);
        }
        return node.asText();
    }

    private static ObjectNode buildOperationsSnapshot(
            RuntimeService runtimeService,
            TaskService taskService,
            ManagementService managementService,
            String processInstanceId,
            String error,
            List<String> observeVariables,
            List<String> observeFields,
            List<String> taskQueryResult) throws IOException {

        boolean processEnded = processInstanceId == null
                || runtimeService.createProcessInstanceQuery()
                        .processInstanceId(processInstanceId)
                        .count() == 0;

        ObjectNode normalized = OBJECT_MAPPER.createObjectNode();
        for (String field : observeFields) {
            switch (field) {
                case "tasks" -> {
                    if (processInstanceId == null || processEnded) {
                        normalized.set("tasks", OBJECT_MAPPER.createArrayNode());
                    } else {
                        normalized.set("tasks", normalizeTasks(taskService, processInstanceId));
                    }
                }
                case "variables" -> {
                    if (processInstanceId == null || processEnded) {
                        normalized.set("variables", OBJECT_MAPPER.createObjectNode());
                    } else {
                        normalized.set(
                                "variables",
                                normalizeVariables(
                                        runtimeService.getVariables(processInstanceId),
                                        observeVariables));
                    }
                }
                case "processEnded" -> normalized.put("processEnded", processEnded);
                case "error" -> {
                    if (error == null) {
                        normalized.putNull("error");
                    } else {
                        normalized.put("error", normalizeContractErrorMessage(error));
                    }
                }
                case "processInstanceCount" -> normalized.put(
                        "processInstanceCount",
                        runtimeService.createProcessInstanceQuery().count());
                case "jobs" -> {
                    ObjectNode jobs = OBJECT_MAPPER.createObjectNode();
                    if (processInstanceId == null) {
                        jobs.put("executable", 0);
                        jobs.put("timer", 0);
                        jobs.put("deadletter", 0);
                    } else {
                        jobs.put(
                                "executable",
                                managementService.createJobQuery()
                                        .processInstanceId(processInstanceId)
                                        .count());
                        jobs.put(
                                "timer",
                                managementService.createTimerJobQuery()
                                        .processInstanceId(processInstanceId)
                                        .count());
                        jobs.put(
                                "deadletter",
                                managementService.createDeadLetterJobQuery()
                                        .processInstanceId(processInstanceId)
                                        .count());
                    }
                    normalized.set("jobs", jobs);
                }
                case "taskLocalVariables" -> {
                    if (processInstanceId == null || processEnded) {
                        normalized.set("taskLocalVariables", OBJECT_MAPPER.createObjectNode());
                    } else {
                        normalized.set(
                                "taskLocalVariables",
                                normalizeTaskLocalVariables(taskService, processInstanceId));
                    }
                }
                case "eventSubprocessTimers" -> {
                    // Java materializes event-subprocess start timers as timer jobs.
                    long count = processInstanceId == null
                            ? 0L
                            : managementService.createTimerJobQuery()
                                    .processInstanceId(processInstanceId)
                                    .count();
                    normalized.put("eventSubprocessTimers", count);
                }
                case "taskDetails" -> {
                    if (processInstanceId == null || processEnded) {
                        normalized.set("taskDetails", OBJECT_MAPPER.createArrayNode());
                    } else {
                        normalized.set(
                                "taskDetails",
                                normalizeTaskDetails(taskService, processInstanceId));
                    }
                }
                case "taskQuery" -> {
                    ArrayNode queryHits = OBJECT_MAPPER.createArrayNode();
                    for (String key : taskQueryResult) {
                        queryHits.add(key);
                    }
                    normalized.set("taskQuery", queryHits);
                }
                case "jobHandlerTypes" -> {
                    ArrayNode types = OBJECT_MAPPER.createArrayNode();
                    if (processInstanceId != null) {
                        List<String> handlerTypes = new ArrayList<>();
                        for (Job job : managementService.createJobQuery()
                                .processInstanceId(processInstanceId)
                                .list()) {
                            if (job.getJobHandlerType() != null) {
                                handlerTypes.add(job.getJobHandlerType());
                            }
                        }
                        Collections.sort(handlerTypes);
                        for (String type : handlerTypes) {
                            types.add(type);
                        }
                    }
                    normalized.set("jobHandlerTypes", types);
                }
                case "timerCalendars" -> {
                    ArrayNode calendars = OBJECT_MAPPER.createArrayNode();
                    if (processInstanceId != null) {
                        List<String> names = new ArrayList<>();
                        for (Job job : managementService.createTimerJobQuery()
                                .processInstanceId(processInstanceId)
                                .list()) {
                            // Java Job entity does not always expose calendarName on the API
                            // surface; fall back to empty string for parity with Rust's
                            // "missing → empty" normalisation when the field is absent.
                            String calendarName = "";
                            try {
                                // Reflective read: some Flowable versions store calendarName
                                // only on the entity, not the public Job interface.
                                java.lang.reflect.Method getter = job.getClass()
                                        .getMethod("getCalendarName");
                                Object value = getter.invoke(job);
                                if (value != null) {
                                    calendarName = value.toString();
                                }
                            } catch (ReflectiveOperationException ignored) {
                                // leave empty
                            }
                            names.add(calendarName);
                        }
                        Collections.sort(names);
                        for (String name : names) {
                            calendars.add(name);
                        }
                    }
                    normalized.set("timerCalendars", calendars);
                }
                default -> throw new IllegalArgumentException("Unknown observe field: " + field);
            }
        }
        return normalized;
    }

    private static ArrayNode normalizeTaskDetails(
            TaskService taskService,
            String processInstanceId) {
        List<Task> tasks = taskService.createTaskQuery()
                .processInstanceId(processInstanceId)
                .list();
        tasks.sort(Comparator.comparing(Task::getTaskDefinitionKey)
                .thenComparing(task -> task.getAssignee() == null ? "" : task.getAssignee())
                .thenComparing(Task::getId));
        ArrayNode details = OBJECT_MAPPER.createArrayNode();
        for (Task task : tasks) {
            ObjectNode node = OBJECT_MAPPER.createObjectNode();
            node.put("taskDefinitionKey", task.getTaskDefinitionKey());
            if (task.getAssignee() == null) {
                node.putNull("assignee");
            } else {
                node.put("assignee", task.getAssignee());
            }
            if (task.getOwner() == null) {
                node.putNull("owner");
            } else {
                node.put("owner", task.getOwner());
            }
            if (task.getDelegationState() == null) {
                node.putNull("delegationState");
            } else {
                node.put("delegationState", task.getDelegationState().name().toLowerCase());
            }
            details.add(node);
        }
        return details;
    }

    private static ObjectNode normalizeTaskLocalVariables(
            TaskService taskService,
            String processInstanceId) throws IOException {
        List<Task> tasks = taskService.createTaskQuery()
                .processInstanceId(processInstanceId)
                .list();
        tasks.sort(Comparator.comparing(Task::getTaskDefinitionKey));
        ObjectNode byTask = OBJECT_MAPPER.createObjectNode();
        for (Task task : tasks) {
            Map<String, Object> locals = taskService.getVariablesLocal(task.getId());
            ObjectNode vars = OBJECT_MAPPER.createObjectNode();
            List<String> names = new ArrayList<>(locals.keySet());
            Collections.sort(names);
            for (String name : names) {
                Object value = locals.get(name);
                if (value != null && !(value instanceof String)
                        && value.getClass().getName().contains("JsonNode")) {
                    vars.set(name, OBJECT_MAPPER.readTree(value.toString()));
                } else {
                    vars.set(name, OBJECT_MAPPER.valueToTree(value));
                }
            }
            byTask.set(task.getTaskDefinitionKey(), vars);
        }
        return byTask;
    }

    private static ObjectNode runCase(
            ProcessEngine processEngine,
            Path fixtureDirectory,
            JsonNode contractCase) throws Exception {

        List<CapturedRequest> capturedRequests = Collections.synchronizedList(new ArrayList<>());
        AtomicInteger responseAttempt = new AtomicInteger();
        boolean needsHttp = contractCase.hasNonNull("path")
                && !contractCase.path("path").asText().isEmpty()
                && contractCase.has("responseStatus");
        HttpServer server = null;
        String endpoint = null;
        if (needsHttp) {
            server = HttpServer.create(new InetSocketAddress("127.0.0.1", 0), 0);
            String expectedPath = contractCase.path("path").asText();
            server.createContext(expectedPath, exchange -> handleRequest(
                    exchange, contractCase, capturedRequests, responseAttempt.getAndIncrement()));
            server.start();
            endpoint = "http://127.0.0.1:" + server.getAddress().getPort() + expectedPath;
        } else if (contractCase.hasNonNull("path")) {
            // Path present only for endpoint injection placeholders (e.g. cancel fixtures).
            endpoint = "http://127.0.0.1:9" + contractCase.path("path").asText();
        }

        try {
            String bpmnName = contractCase.path("bpmn").asText();
            String bpmnXml = Files.readString(fixtureDirectory.resolve(bpmnName), StandardCharsets.UTF_8);

            RepositoryService repositoryService = processEngine.getRepositoryService();
            RuntimeService runtimeService = processEngine.getRuntimeService();
            TaskService taskService = processEngine.getTaskService();
            Deployment deployment = repositoryService.createDeployment()
                    .name("contract-" + contractCase.path("id").asText())
                    .addString(bpmnName, bpmnXml)
                    .deploy();

            String processDefinitionId = repositoryService.createProcessDefinitionQuery()
                    .deploymentId(deployment.getId())
                    .singleResult()
                    .getId();
            String execution = contractCase.path("execution").asText("sync");
            List<String> observeVariables = resolveObserveVariables(contractCase);
            ProcessInstance processInstance;
            try {
                Map<String, Object> startVariables = new java.util.LinkedHashMap<>();
                if (endpoint != null) {
                    startVariables.put("endpoint", endpoint);
                }
                processInstance = runtimeService.startProcessInstanceById(
                        processDefinitionId,
                        startVariables);
            } catch (RuntimeException exception) {
                if (!"syncObserved".equals(execution)) {
                    throw exception;
                }
                return normalizeObservedSyncCase(
                        runtimeService,
                        taskService,
                        processDefinitionId,
                        null,
                        capturedRequests,
                        exception.getMessage(),
                        observeVariables);
            }

            if ("sync".equals(execution)) {
                ObjectNode normalized = OBJECT_MAPPER.createObjectNode();
                if (!capturedRequests.isEmpty()) {
                    normalized.set("request", normalizeRequest(capturedRequests.get(0)));
                } else {
                    normalized.putNull("request");
                }
                normalized.set(
                        "variables",
                        normalizeVariables(
                                runtimeService.getVariables(processInstance.getId()),
                                observeVariables));
                normalized.set("tasks", normalizeTasks(taskService, processInstance.getId()));
                normalized.putNull("error");
                return normalized;
            }
            if ("syncObserved".equals(execution)) {
                return normalizeObservedSyncCase(
                        runtimeService,
                        taskService,
                        processDefinitionId,
                        processInstance,
                        capturedRequests,
                        null,
                        observeVariables);
            }
            if (execution.endsWith("Cancel")) {
                return runCancelCase(
                        processEngine,
                        processInstance.getId(),
                        execution,
                        capturedRequests);
            }
            return runAsyncCase(
                    processEngine,
                    processInstance.getId(),
                    execution,
                    capturedRequests,
                    observeVariables);
        } finally {
            if (server != null) {
                server.stop(0);
            }
        }
    }

    private static ObjectNode normalizeObservedSyncCase(
            RuntimeService runtimeService,
            TaskService taskService,
            String processDefinitionId,
            ProcessInstance processInstance,
            List<CapturedRequest> capturedRequests,
            String error,
            List<String> observeVariables) throws IOException {

        ObjectNode normalized = OBJECT_MAPPER.createObjectNode();
        normalized.put("requestCount", capturedRequests.size());
        if (capturedRequests.isEmpty()) {
            normalized.putNull("request");
        } else {
            normalized.set("request", normalizeRequest(capturedRequests.get(0)));
        }
        normalized.put(
                "processInstanceCount",
                runtimeService.createProcessInstanceQuery()
                        .processDefinitionId(processDefinitionId)
                        .count());
        if (processInstance == null) {
            normalized.set("variables", OBJECT_MAPPER.createObjectNode());
            normalized.set("tasks", OBJECT_MAPPER.createArrayNode());
        } else {
            normalized.set(
                    "variables",
                    normalizeVariables(
                            runtimeService.getVariables(processInstance.getId()),
                            observeVariables));
            normalized.set("tasks", normalizeTasks(taskService, processInstance.getId()));
        }
        if (error == null) {
            normalized.putNull("error");
        } else {
            normalized.put("error", error);
        }
        return normalized;
    }

    private static ObjectNode runUnlockOwnedJobsCase(
            Path fixtureDirectory,
            JsonNode contractCase) throws Exception {

        ObjectNode normalized = OBJECT_MAPPER.createObjectNode();
        normalized.put("defaultUnlockOwnedJobs", true);
        normalized.set(
                "defaultPolicy",
                runUnlockOwnedJobsPolicy(fixtureDirectory, contractCase, null));
        normalized.set(
                "explicitFalse",
                runUnlockOwnedJobsPolicy(fixtureDirectory, contractCase, false));
        return normalized;
    }

    private static ObjectNode runUnlockOwnedJobsPolicy(
            Path fixtureDirectory,
            JsonNode contractCase,
            Boolean unlockOwnedJobs) throws Exception {

        String policyName = unlockOwnedJobs == null ? "default" : "disabled";
        ProcessEngineConfigurationImpl configuration =
                (ProcessEngineConfigurationImpl) ProcessEngineConfiguration
                        .createStandaloneInMemProcessEngineConfiguration();
        configuration.setEngineName("java-unlock-owned-jobs-" + policyName);
        configuration.setJdbcUrl(
                "jdbc:h2:mem:java-unlock-owned-jobs-" + policyName + "-"
                        + System.nanoTime() + ";DB_CLOSE_DELAY=-1");
        configuration.setDatabaseSchemaUpdate(ProcessEngineConfiguration.DB_SCHEMA_UPDATE_TRUE);
        configuration.setAsyncExecutorActivate(false);
        configuration.setAsyncExecutorLockOwner(UNLOCK_OWNED_JOBS_LOCK_OWNER);
        configuration.getAsyncExecutorConfiguration().setAsyncJobAcquisitionEnabled(false);
        configuration.getAsyncExecutorConfiguration().setTimerJobAcquisitionEnabled(false);
        configuration.getAsyncExecutorConfiguration().setResetExpiredJobEnabled(false);
        if (unlockOwnedJobs != null) {
            configuration.getAsyncExecutorConfiguration().setUnlockOwnedJobs(unlockOwnedJobs);
        }

        ProcessEngine engine = configuration.buildProcessEngine();
        try {
            DefaultAsyncJobExecutor executor =
                    (DefaultAsyncJobExecutor) configuration.getAsyncExecutor();
            String processDefinitionId = deployUnlockOwnedJobsFixture(
                    engine,
                    fixtureDirectory,
                    contractCase);

            boolean configuredUnlockOwnedJobs =
                    executor.getConfiguration().isUnlockOwnedJobs();
            String shutdownProcessInstanceId = engine.getRuntimeService()
                    .startProcessInstanceById(processDefinitionId)
                    .getId();
            executor.start();
            acquireJavaJob(configuration, executor, shutdownProcessInstanceId);
            JobSnapshot shutdownBefore = snapshotJavaJob(
                    engine.getManagementService(),
                    shutdownProcessInstanceId);
            boolean shutdownActiveBefore = executor.isActive();
            executor.shutdown();
            JobSnapshot shutdownAfter = snapshotJavaJob(
                    engine.getManagementService(),
                    shutdownProcessInstanceId);

            String startupProcessInstanceId = engine.getRuntimeService()
                    .startProcessInstanceById(processDefinitionId)
                    .getId();
            acquireJavaJob(configuration, executor, startupProcessInstanceId);
            JobSnapshot startupBefore = snapshotJavaJob(
                    engine.getManagementService(),
                    startupProcessInstanceId);
            boolean startupActiveBefore = executor.isActive();
            executor.start();
            JobSnapshot startupAfter = snapshotJavaJob(
                    engine.getManagementService(),
                    startupProcessInstanceId);

            ObjectNode normalized = OBJECT_MAPPER.createObjectNode();
            normalized.put("configuredUnlockOwnedJobs", configuredUnlockOwnedJobs);
            normalized.set(
                    "shutdown",
                    normalizeJavaUnlockTransition(
                            shutdownBefore,
                            shutdownAfter,
                            shutdownActiveBefore,
                            false));
            normalized.set(
                    "startup",
                    normalizeJavaUnlockTransition(
                            startupBefore,
                            startupAfter,
                            startupActiveBefore,
                            executor.isActive()));
            return normalized;
        } finally {
            engine.close();
        }
    }

    private static String deployUnlockOwnedJobsFixture(
            ProcessEngine engine,
            Path fixtureDirectory,
            JsonNode contractCase) throws IOException {

        String bpmnName = contractCase.path("bpmn").asText();
        String bpmnXml = Files.readString(
                fixtureDirectory.resolve(bpmnName),
                StandardCharsets.UTF_8);
        Deployment deployment = engine.getRepositoryService()
                .createDeployment()
                .name("contract-" + contractCase.path("id").asText())
                .addString(bpmnName, bpmnXml)
                .deploy();
        return engine.getRepositoryService()
                .createProcessDefinitionQuery()
                .deploymentId(deployment.getId())
                .singleResult()
                .getId();
    }

    private static void acquireJavaJob(
            ProcessEngineConfigurationImpl configuration,
            DefaultAsyncJobExecutor executor,
            String processInstanceId) {

        List<? extends JobInfoEntity> acquired = configuration.getCommandExecutor()
                .execute(new AcquireJobsCmd(executor));
        boolean targetAcquired = acquired.stream()
                .filter(Job.class::isInstance)
                .map(Job.class::cast)
                .anyMatch(job -> processInstanceId.equals(job.getProcessInstanceId()));
        if (!targetAcquired) {
            throw new IllegalStateException(
                    "AcquireJobsCmd did not lock the expected process job " + processInstanceId);
        }
    }

    private static JobSnapshot snapshotJavaJob(
            ManagementService managementService,
            String processInstanceId) {

        Job job = requireExecutableJob(
                managementService,
                processInstanceId,
                "unlockOwnedJobs lifecycle observation");
        JobInfoEntity jobInfo = requireJobInfoEntity(job);
        return new JobSnapshot(
                job.getId(),
                job.getProcessInstanceId(),
                job.getExecutionId(),
                job.getProcessDefinitionId(),
                job.getElementId(),
                job.getElementName(),
                job.getCategory(),
                job.getJobType(),
                job.getTenantId(),
                job.getJobHandlerType(),
                job.getJobHandlerConfiguration(),
                job.getCustomValues(),
                job.getExceptionMessage(),
                job.isExclusive(),
                job.getRetries(),
                job.getDuedate(),
                jobInfo.getLockOwner(),
                jobInfo.getLockExpirationTime());
    }

    private static ObjectNode normalizeJavaUnlockTransition(
            JobSnapshot before,
            JobSnapshot after,
            boolean activeBefore,
            boolean activeAfter) {

        ObjectNode normalized = OBJECT_MAPPER.createObjectNode();
        normalized.put("activeBefore", activeBefore);
        normalized.put("activeAfter", activeAfter);
        normalized.set("before", normalizeJavaLockState(before));
        normalized.set("after", normalizeJavaLockState(after));
        normalized.put("stateUnchanged", true);
        normalized.put("retriesUnchanged", before.retries() == after.retries());
        normalized.put("dueDateUnchanged", Objects.equals(before.dueDate(), after.dueDate()));
        normalized.put("otherFieldsUnchanged", before.sameNonLockFields(after));
        return normalized;
    }

    private static ObjectNode normalizeJavaLockState(JobSnapshot snapshot) {
        ObjectNode normalized = OBJECT_MAPPER.createObjectNode();
        normalized.put("state", "executable");
        normalized.put("retries", snapshot.retries());
        if (snapshot.lockOwner() == null) {
            normalized.putNull("lockOwner");
        } else {
            normalized.put("lockOwner", snapshot.lockOwner());
        }
        normalized.put("lockExpirationSet", snapshot.lockExpiration() != null);
        return normalized;
    }

    private static ObjectNode runSharedMultiTenantUnlockOwnedJobsCase(
            Path fixtureDirectory,
            JsonNode contractCase) throws Exception {

        ContractTenantInfoHolder tenantInfoHolder = new ContractTenantInfoHolder(
                SHARED_REGISTERED_TENANTS);
        SharedExecutorServiceAsyncExecutor sharedDefaults =
                new SharedExecutorServiceAsyncExecutor(tenantInfoHolder);

        ObjectNode normalized = OBJECT_MAPPER.createObjectNode();
        normalized.put(
                "engineDefaultUnlockOwnedJobs",
                new AsyncJobExecutorConfiguration().isUnlockOwnedJobs());
        normalized.put(
                "sharedDefaultUnlockOwnedJobs",
                sharedDefaults.getConfiguration().isUnlockOwnedJobs());
        normalized.set("registeredTenants", OBJECT_MAPPER.valueToTree(SHARED_REGISTERED_TENANTS));
        normalized.set(
                "defaultFalse",
                runSharedMultiTenantUnlockOwnedJobsPolicy(
                        fixtureDirectory,
                        contractCase,
                        false,
                        "default-false"));
        normalized.set(
                "explicitTrue",
                runSharedMultiTenantUnlockOwnedJobsPolicy(
                        fixtureDirectory,
                        contractCase,
                        true,
                        "explicit-true"));
        return normalized;
    }

    private static ObjectNode runSharedMultiTenantUnlockOwnedJobsPolicy(
            Path fixtureDirectory,
            JsonNode contractCase,
            boolean unlockOwnedJobs,
            String policyName) throws Exception {

        ContractTenantInfoHolder tenantInfoHolder = new ContractTenantInfoHolder(
                SHARED_REGISTERED_TENANTS);
        SharedExecutorServiceAsyncExecutor executor =
                new SharedExecutorServiceAsyncExecutor(tenantInfoHolder);
        executor.setLockOwner(UNLOCK_OWNED_JOBS_LOCK_OWNER);
        executor.setUnlockOwnedJobs(unlockOwnedJobs);
        executor.setMaxAsyncJobsDuePerAcquisition(1);
        executor.setAsyncJobAcquisitionEnabled(false);
        executor.setTimerJobAcquisitionEnabled(false);
        executor.setResetExpiredJobEnabled(false);

        ProcessEngineConfigurationImpl configuration =
                (ProcessEngineConfigurationImpl) ProcessEngineConfiguration
                        .createStandaloneInMemProcessEngineConfiguration();
        configuration.setEngineName("java-shared-unlock-owned-jobs-" + policyName);
        configuration.setJdbcUrl(
                "jdbc:h2:mem:java-shared-unlock-owned-jobs-" + policyName + "-"
                        + System.nanoTime() + ";DB_CLOSE_DELAY=-1");
        configuration.setDatabaseSchemaUpdate(ProcessEngineConfiguration.DB_SCHEMA_UPDATE_TRUE);
        configuration.setAsyncExecutorActivate(false);
        configuration.setAsyncExecutor(executor);

        ProcessEngine engine = configuration.buildProcessEngine();
        try {
            for (String tenantId : SHARED_REGISTERED_TENANTS) {
                executor.addTenantAsyncExecutor(tenantId, false);
            }
            SharedUnlockProcessDefinitions processDefinitions =
                    deployJavaSharedUnlockOwnedJobsFixtures(
                    engine,
                    fixtureDirectory,
                    contractCase);
            SharedUnlockProcessInstances processInstances =
                    createAndAcquireJavaSharedUnlockJobs(
                            engine,
                            configuration,
                            executor,
                            processDefinitions);

            SharedUnlockJobSnapshots beforeStart = snapshotJavaSharedUnlockJobs(
                    engine.getManagementService(),
                    processInstances);
            boolean startupActiveBefore = executor.isActive();
            executor.start();
            SharedUnlockJobSnapshots afterStart = snapshotJavaSharedUnlockJobs(
                    engine.getManagementService(),
                    processInstances);

            boolean shutdownActiveBefore = executor.isActive();
            executor.shutdown();
            SharedUnlockJobSnapshots afterShutdown = snapshotJavaSharedUnlockJobs(
                    engine.getManagementService(),
                    processInstances);

            ObjectNode normalized = OBJECT_MAPPER.createObjectNode();
            normalized.put(
                    "configuredUnlockOwnedJobs",
                    executor.getConfiguration().isUnlockOwnedJobs());
            normalized.set(
                    "startup",
                    normalizeJavaSharedUnlockPhase(
                            beforeStart,
                            afterStart,
                            startupActiveBefore,
                            true));
            normalized.set(
                    "shutdown",
                    normalizeJavaSharedUnlockPhase(
                            afterStart,
                            afterShutdown,
                            shutdownActiveBefore,
                            false));
            return normalized;
        } finally {
            engine.close();
        }
    }

    private static SharedUnlockProcessInstances createAndAcquireJavaSharedUnlockJobs(
            ProcessEngine engine,
            ProcessEngineConfigurationImpl configuration,
            SharedExecutorServiceAsyncExecutor executor,
            SharedUnlockProcessDefinitions processDefinitions) {

        String registeredTenantA = startJavaUnlockOwnedJobsProcess(
                engine,
                processDefinitions.tenantA());
        acquireJavaJob(configuration, executor, registeredTenantA);
        String registeredTenantB = startJavaUnlockOwnedJobsProcess(
                engine,
                processDefinitions.tenantB());
        acquireJavaJob(configuration, executor, registeredTenantB);
        String unregisteredTenantC = startJavaUnlockOwnedJobsProcess(
                engine,
                processDefinitions.tenantC());
        acquireJavaJob(configuration, executor, unregisteredTenantC);

        String otherOwnerTenantA = startJavaUnlockOwnedJobsProcess(
                engine,
                processDefinitions.tenantA());
        executor.setLockOwner(SHARED_UNLOCK_OTHER_OWNER);
        try {
            acquireJavaJob(configuration, executor, otherOwnerTenantA);
        } finally {
            executor.setLockOwner(UNLOCK_OWNED_JOBS_LOCK_OWNER);
        }

        return new SharedUnlockProcessInstances(
                registeredTenantA,
                registeredTenantB,
                unregisteredTenantC,
                otherOwnerTenantA);
    }

    private static String startJavaUnlockOwnedJobsProcess(
            ProcessEngine engine,
            String processDefinitionId) {

        return engine.getRuntimeService()
                .startProcessInstanceById(processDefinitionId)
                .getId();
    }

    private static SharedUnlockProcessDefinitions deployJavaSharedUnlockOwnedJobsFixtures(
            ProcessEngine engine,
            Path fixtureDirectory,
            JsonNode contractCase) throws IOException {

        return new SharedUnlockProcessDefinitions(
                deployJavaSharedUnlockOwnedJobsFixture(
                        engine,
                        fixtureDirectory,
                        contractCase,
                        SHARED_TENANT_A),
                deployJavaSharedUnlockOwnedJobsFixture(
                        engine,
                        fixtureDirectory,
                        contractCase,
                        SHARED_TENANT_B),
                deployJavaSharedUnlockOwnedJobsFixture(
                        engine,
                        fixtureDirectory,
                        contractCase,
                        SHARED_TENANT_C));
    }

    private static String deployJavaSharedUnlockOwnedJobsFixture(
            ProcessEngine engine,
            Path fixtureDirectory,
            JsonNode contractCase,
            String tenantId) throws IOException {

        String bpmnName = contractCase.path("bpmn").asText();
        String bpmnXml = Files.readString(
                fixtureDirectory.resolve(bpmnName),
                StandardCharsets.UTF_8);
        Deployment deployment = engine.getRepositoryService()
                .createDeployment()
                .name("contract-" + contractCase.path("id").asText() + "-" + tenantId)
                .tenantId(tenantId)
                .addString(bpmnName, bpmnXml)
                .deploy();
        return engine.getRepositoryService()
                .createProcessDefinitionQuery()
                .deploymentId(deployment.getId())
                .singleResult()
                .getId();
    }

    private static SharedUnlockJobSnapshots snapshotJavaSharedUnlockJobs(
            ManagementService managementService,
            SharedUnlockProcessInstances processInstances) {

        return new SharedUnlockJobSnapshots(
                snapshotJavaJob(managementService, processInstances.registeredTenantA()),
                snapshotJavaJob(managementService, processInstances.registeredTenantB()),
                snapshotJavaJob(managementService, processInstances.unregisteredTenantC()),
                snapshotJavaJob(managementService, processInstances.otherOwnerTenantA()));
    }

    private static ObjectNode normalizeJavaSharedUnlockPhase(
            SharedUnlockJobSnapshots before,
            SharedUnlockJobSnapshots after,
            boolean activeBefore,
            boolean activeAfter) {

        ObjectNode normalized = OBJECT_MAPPER.createObjectNode();
        normalized.set(
                "registeredTenantA",
                normalizeJavaUnlockTransition(
                        before.registeredTenantA(),
                        after.registeredTenantA(),
                        activeBefore,
                        activeAfter));
        normalized.set(
                "registeredTenantB",
                normalizeJavaUnlockTransition(
                        before.registeredTenantB(),
                        after.registeredTenantB(),
                        activeBefore,
                        activeAfter));
        normalized.set(
                "unregisteredTenantC",
                normalizeJavaUnlockTransition(
                        before.unregisteredTenantC(),
                        after.unregisteredTenantC(),
                        activeBefore,
                        activeAfter));
        normalized.set(
                "otherOwnerTenantA",
                normalizeJavaUnlockTransition(
                        before.otherOwnerTenantA(),
                        after.otherOwnerTenantA(),
                        activeBefore,
                        activeAfter));
        return normalized;
    }

    private record SharedUnlockProcessInstances(
            String registeredTenantA,
            String registeredTenantB,
            String unregisteredTenantC,
            String otherOwnerTenantA) {
    }

    private record SharedUnlockProcessDefinitions(
            String tenantA,
            String tenantB,
            String tenantC) {
    }

    private record SharedUnlockJobSnapshots(
            JobSnapshot registeredTenantA,
            JobSnapshot registeredTenantB,
            JobSnapshot unregisteredTenantC,
            JobSnapshot otherOwnerTenantA) {
    }

    private static final class ContractTenantInfoHolder implements TenantInfoHolder {

        private final Set<String> tenants;
        private final ThreadLocal<String> currentTenantId = new ThreadLocal<>();

        private ContractTenantInfoHolder(Collection<String> tenants) {
            this.tenants = new LinkedHashSet<>(tenants);
        }

        @Override
        public Collection<String> getAllTenants() {
            return Collections.unmodifiableSet(tenants);
        }

        @Override
        public void setCurrentTenantId(String tenantId) {
            currentTenantId.set(tenantId);
        }

        @Override
        public String getCurrentTenantId() {
            return currentTenantId.get();
        }

        @Override
        public void clearCurrentTenantId() {
            currentTenantId.remove();
        }
    }

    private record JobSnapshot(
            String id,
            String processInstanceId,
            String executionId,
            String processDefinitionId,
            String elementId,
            String elementName,
            String category,
            String jobType,
            String tenantId,
            String jobHandlerType,
            String jobHandlerConfiguration,
            String customValues,
            String exceptionMessage,
            boolean exclusive,
            int retries,
            Date dueDate,
            String lockOwner,
            Date lockExpiration) {

        boolean sameNonLockFields(JobSnapshot other) {
            return Objects.equals(id, other.id)
                    && Objects.equals(processInstanceId, other.processInstanceId)
                    && Objects.equals(executionId, other.executionId)
                    && Objects.equals(processDefinitionId, other.processDefinitionId)
                    && Objects.equals(elementId, other.elementId)
                    && Objects.equals(elementName, other.elementName)
                    && Objects.equals(category, other.category)
                    && Objects.equals(jobType, other.jobType)
                    && Objects.equals(tenantId, other.tenantId)
                    && Objects.equals(jobHandlerType, other.jobHandlerType)
                    && Objects.equals(jobHandlerConfiguration, other.jobHandlerConfiguration)
                    && Objects.equals(customValues, other.customValues)
                    && Objects.equals(exceptionMessage, other.exceptionMessage)
                    && exclusive == other.exclusive;
        }
    }

    private static ObjectNode runAutomaticAsyncRetryCase(
            Path fixtureDirectory,
            JsonNode contractCase) throws Exception {

        GatedContractServer server = new GatedContractServer(contractCase);
        ProcessEngine processEngine = null;
        try {
            ProcessEngineConfigurationImpl configuration =
                    (ProcessEngineConfigurationImpl) ProcessEngineConfiguration
                            .createStandaloneInMemProcessEngineConfiguration();
            configuration.setEngineName("java-http-differential-auto");
            configuration.setJdbcUrl(
                    "jdbc:h2:mem:flowable-java-http-differential-auto;DB_CLOSE_DELAY=-1");
            configuration.setDatabaseSchemaUpdate(ProcessEngineConfiguration.DB_SCHEMA_UPDATE_TRUE);
            configuration.setAsyncExecutorActivate(true);
            configuration.setAsyncExecutorNumberOfRetries(3);
            configuration.setAsyncExecutorDefaultAsyncJobAcquireWaitTime(25);
            configuration.setAsyncExecutorDefaultTimerJobAcquireWaitTime(25);
            configuration.setAsyncExecutorDefaultQueueSizeFullWaitTime(25);
            configuration.setAsyncExecutorMaxAsyncJobsDuePerAcquisition(1);
            configuration.setAsyncExecutorMaxTimerJobsPerAcquisition(1);
            configuration.setAsyncExecutorLockOwner(AUTOMATIC_EXECUTOR_LOCK_OWNER);
            configuration.setAsyncExecutorAsyncJobLockTimeInMillis(5_000);
            configuration.setAsyncExecutorTimerLockTimeInMillis(5_000);
            DefaultClockImpl fixedClock = new DefaultClockImpl();
            fixedClock.setCurrentTime(new Date(FIXED_CLOCK_MILLIS));
            configuration.setClock(fixedClock);
            processEngine = configuration.buildProcessEngine();
            ProcessEngine automaticEngine = processEngine;
            RecordingJobEventListener eventListener = new RecordingJobEventListener();
            automaticEngine.getProcessEngineConfiguration()
                    .getEventDispatcher()
                    .addEventListener(eventListener);

            String bpmnName = contractCase.path("bpmn").asText();
            String bpmnXml = Files.readString(
                    fixtureDirectory.resolve(bpmnName),
                    StandardCharsets.UTF_8);
            RepositoryService repositoryService = automaticEngine.getRepositoryService();
            Deployment deployment = repositoryService.createDeployment()
                    .name("contract-" + contractCase.path("id").asText())
                    .addString(bpmnName, bpmnXml)
                    .deploy();
            String processDefinitionId = repositoryService.createProcessDefinitionQuery()
                    .deploymentId(deployment.getId())
                    .singleResult()
                    .getId();
            ProcessInstance processInstance = automaticEngine.getRuntimeService()
                    .startProcessInstanceById(
                            processDefinitionId,
                            Map.of("endpoint", server.endpoint()));

            server.awaitRequest(0);
            Job firstJob = requireExecutableJob(
                    automaticEngine.getManagementService(),
                    processInstance.getId(),
                    "first automatic acquisition");
            ObjectNode firstAcquisition = normalizeAcquisition(
                    firstJob,
                    fixedClock.getCurrentTime());
            server.allowResponse(0);

            Job retryTimer = waitForValue(
                    "automatic retry timer",
                    () -> automaticEngine.getManagementService()
                            .createTimerJobQuery()
                            .processInstanceId(processInstance.getId())
                            .singleResult());
            Date retryObservationTime = fixedClock.getCurrentTime();
            ObjectNode retryTimerNode = normalizeRetryTimer(retryTimer, retryObservationTime);

            fixedClock.setCurrentTime(new Date(
                    retryObservationTime.getTime() + ASYNC_RETRY_ADVANCE_MILLIS));
            server.awaitRequest(1);
            Job secondJob = requireExecutableJob(
                    automaticEngine.getManagementService(),
                    processInstance.getId(),
                    "second automatic acquisition");
            ObjectNode secondAcquisition = normalizeAcquisition(
                    secondJob,
                    fixedClock.getCurrentTime());
            server.allowResponse(1);

            waitForCondition(
                    "review task after automatic retry",
                    () -> automaticEngine.getTaskService()
                            .createTaskQuery()
                            .processInstanceId(processInstance.getId())
                            .taskDefinitionKey("review")
                            .count() == 1);
            waitForCondition(
                    "automatic retry job consumption",
                    () -> "consumed".equals(normalizeFinalJobState(
                            automaticEngine.getManagementService(),
                            processInstance.getId())));
            server.throwIfHandlerFailed();

            ObjectNode normalized = OBJECT_MAPPER.createObjectNode();
            normalized.put(
                    "executorActive",
                    automaticEngine.getProcessEngineConfiguration()
                            .getAsyncExecutor()
                            .isActive());
            normalized.put("requestCount", server.requestCount());
            normalized.set("firstAcquisition", firstAcquisition);
            normalized.set("retryTimer", retryTimerNode);
            normalized.set("secondAcquisition", secondAcquisition);
            normalized.put(
                    "finalJobState",
                    normalizeFinalJobState(
                            automaticEngine.getManagementService(),
                            processInstance.getId()));
            normalized.set("events", OBJECT_MAPPER.valueToTree(eventListener.events()));
            normalized.set(
                    "tasks",
                    normalizeTasks(automaticEngine.getTaskService(), processInstance.getId()));
            return normalized;

        } finally {
            server.releaseAllResponses();
            try {
                if (processEngine != null) {
                    processEngine.close();
                }
            } finally {
                server.close();
            }
        }
    }

    private static Job requireExecutableJob(
            ManagementService managementService,
            String processInstanceId,
            String phase) {

        Job job = managementService.createJobQuery()
                .processInstanceId(processInstanceId)
                .singleResult();
        if (job == null) {
            throw new IllegalStateException(phase + " did not expose an executable Job");
        }
        return job;
    }

    private static ObjectNode normalizeAcquisition(Job job, Date currentTime) {
        JobInfoEntity jobInfo = requireJobInfoEntity(job);
        ObjectNode normalized = OBJECT_MAPPER.createObjectNode();
        normalized.put("automatic", true);
        normalized.put("jobState", "executable");
        normalized.put("retries", job.getRetries());
        normalized.put("lockOwnerSet", jobInfo.getLockOwner() != null);
        normalized.put(
                "lockOwnerMatchesConfigured",
                AUTOMATIC_EXECUTOR_LOCK_OWNER.equals(jobInfo.getLockOwner()));
        Date lockExpiration = jobInfo.getLockExpirationTime();
        normalized.put("lockExpirationSet", lockExpiration != null);
        if (lockExpiration == null) {
            normalized.putNull("lockDurationMillis");
        } else {
            normalized.put(
                    "lockDurationMillis",
                    lockExpiration.getTime() - currentTime.getTime());
        }
        return normalized;
    }

    private static ObjectNode normalizeRetryTimer(Job retryTimer, Date currentTime) {
        JobInfoEntity jobInfo = requireJobInfoEntity(retryTimer);
        Date dueDate = retryTimer.getDuedate();
        ObjectNode normalized = OBJECT_MAPPER.createObjectNode();
        normalized.put("visible", true);
        normalized.put("dueDateSet", dueDate != null);
        normalized.put(
                "dueAfterCurrentTime",
                dueDate != null && dueDate.after(currentTime));
        if (dueDate == null) {
            normalized.putNull("retryDelayMillis");
        } else {
            normalized.put("retryDelayMillis", dueDate.getTime() - currentTime.getTime());
        }
        normalized.put("retries", retryTimer.getRetries());
        if (retryTimer.getExceptionMessage() == null) {
            normalized.putNull("errorMessage");
        } else {
            normalized.put("errorMessage", retryTimer.getExceptionMessage());
        }
        normalized.put("lockOwnerSet", jobInfo.getLockOwner() != null);
        normalized.put("lockExpirationSet", jobInfo.getLockExpirationTime() != null);
        return normalized;
    }

    private static JobInfoEntity requireJobInfoEntity(Job job) {
        if (!(job instanceof JobInfoEntity jobInfo)) {
            throw new IllegalStateException(
                    "Flowable Job implementation does not expose JobInfoEntity lock state: "
                            + job.getClass().getName());
        }
        return jobInfo;
    }

    private static String normalizeFinalJobState(
            ManagementService managementService,
            String processInstanceId) {

        if (managementService.createJobQuery().processInstanceId(processInstanceId).count() > 0) {
            return "executable";
        }
        if (managementService.createTimerJobQuery().processInstanceId(processInstanceId).count() > 0) {
            return "timer";
        }
        if (managementService.createDeadLetterJobQuery().processInstanceId(processInstanceId).count() > 0) {
            return "deadletter";
        }
        return "consumed";
    }

    private static <T> T waitForValue(String description, Supplier<T> supplier) {
        long deadline = System.nanoTime() + TimeUnit.MILLISECONDS.toNanos(WALL_TIMEOUT_MILLIS);
        while (System.nanoTime() < deadline) {
            T value = supplier.get();
            if (value != null) {
                return value;
            }
            sleepForPolling(description);
        }
        throw new IllegalStateException("Timed out waiting for " + description);
    }

    private static void waitForCondition(String description, BooleanSupplier condition) {
        long deadline = System.nanoTime() + TimeUnit.MILLISECONDS.toNanos(WALL_TIMEOUT_MILLIS);
        while (System.nanoTime() < deadline) {
            if (condition.getAsBoolean()) {
                return;
            }
            sleepForPolling(description);
        }
        throw new IllegalStateException("Timed out waiting for " + description);
    }

    private static void sleepForPolling(String description) {
        try {
            Thread.sleep(10L);
        } catch (InterruptedException exception) {
            Thread.currentThread().interrupt();
            throw new IllegalStateException("Interrupted while waiting for " + description, exception);
        }
    }

    private static ObjectNode runCancelCase(
            ProcessEngine processEngine,
            String processInstanceId,
            String execution,
            List<CapturedRequest> capturedRequests) {

        ManagementService managementService = processEngine.getManagementService();
        Job initialJob = managementService.createJobQuery()
                .processInstanceId(processInstanceId)
                .singleResult();
        if (initialJob == null) {
            throw new IllegalStateException("Cancel contract case did not create an executable job");
        }

        List<String> phases = Collections.synchronizedList(new ArrayList<>());
        List<TransactionRecordingJobEventListener> listeners = new ArrayList<>();
        listeners.add(new TransactionRecordingJobEventListener("IMMEDIATE", null, false, phases));
        for (String phase : List.of("COMMITTING", "COMMITTED", "ROLLINGBACK", "ROLLED_BACK")) {
            listeners.add(new TransactionRecordingJobEventListener(phase, phase, false, phases));
        }
        if ("fatalCommittingCancel".equals(execution)) {
            listeners.add(new TransactionRecordingJobEventListener(
                    "COMMITTING", "COMMITTING", true, phases));
        } else if ("fatalCommittedCancel".equals(execution)) {
            listeners.add(new TransactionRecordingJobEventListener(
                    "COMMITTED", "COMMITTED", true, phases));
        }
        listeners.forEach(listener -> processEngine.getProcessEngineConfiguration()
                .getEventDispatcher().addEventListener(listener));

        String commandError = null;
        try {
            managementService.deleteJob(initialJob.getId());
        } catch (RuntimeException exception) {
            commandError = deepestMessage(exception);
        } finally {
            listeners.forEach(listener -> processEngine.getProcessEngineConfiguration()
                    .getEventDispatcher().removeEventListener(listener));
        }

        boolean jobExists = managementService.createJobQuery().jobId(initialJob.getId()).count() > 0
                || managementService.createTimerJobQuery().jobId(initialJob.getId()).count() > 0
                || managementService.createDeadLetterJobQuery().jobId(initialJob.getId()).count() > 0;
        ObjectNode normalized = OBJECT_MAPPER.createObjectNode();
        normalized.put("requestCount", capturedRequests.size());
        normalized.set("phases", OBJECT_MAPPER.valueToTree(phases));
        if (commandError == null) {
            normalized.putNull("commandError");
        } else {
            normalized.put("commandError", commandError);
        }
        normalized.put("jobState", jobExists ? "executable" : "deleted");
        normalized.putNull("error");
        return normalized;
    }

    private static String deepestMessage(Throwable throwable) {
        Throwable current = throwable;
        while (current.getCause() != null) {
            current = current.getCause();
        }
        return current.getMessage();
    }

    /**
     * Strip engine-specific execution/definition IDs from well-known contract errors
     * so Java and Rust compare on stable semantic text.
     */
    private static String normalizeContractErrorMessage(String error) {
        if (error == null) {
            return null;
        }
        // Exclusive gateway no-outgoing: keep up to "could be selected".
        String marker = "could be selected";
        if (error.contains("No outgoing sequence flow of the exclusive gateway")
                && error.contains(marker)) {
            int end = error.indexOf(marker) + marker.length();
            return error.substring(0, end);
        }
        // Empty timer configuration (P26): keep the stable prefix only.
        String timerMarker =
                "Timer needs configuration (either timeDate, timeCycle or timeDuration is needed)";
        if (error.contains(timerMarker)) {
            return timerMarker;
        }
        return error;
    }

    private static ObjectNode runAsyncCase(
            ProcessEngine processEngine,
            String processInstanceId,
            String execution,
            List<CapturedRequest> capturedRequests,
            List<String> observeVariables) throws IOException {

        ManagementService managementService = processEngine.getManagementService();
        RuntimeService runtimeService = processEngine.getRuntimeService();
        TaskService taskService = processEngine.getTaskService();
        Job initialJob = managementService.createJobQuery()
                .processInstanceId(processInstanceId)
                .singleResult();
        if (initialJob == null) {
            throw new IllegalStateException("Async contract case did not create an executable job");
        }

        RecordingJobEventListener eventListener = new RecordingJobEventListener();
        processEngine.getProcessEngineConfiguration().getEventDispatcher().addEventListener(eventListener);
        ArrayNode attempts = OBJECT_MAPPER.createArrayNode();
        try {
            boolean nestedUnrecoverable = "nestedUnrecoverable".equals(execution);
            attempts.add(executeJobAttempt(
                    managementService,
                    initialJob.getId(),
                    nestedUnrecoverable));
            if ("asyncRetry".equals(execution)) {
                Job timerJob = managementService.createTimerJobQuery()
                        .jobId(initialJob.getId())
                        .singleResult();
                if (timerJob == null) {
                    throw new IllegalStateException("Failed async job was not moved to a Java timer job");
                }
                managementService.moveTimerToExecutableJob(initialJob.getId());
                attempts.add(executeJobAttempt(managementService, initialJob.getId(), false));
            }
        } finally {
            processEngine.getProcessEngineConfiguration().getEventDispatcher().removeEventListener(eventListener);
        }

        ObjectNode normalized = OBJECT_MAPPER.createObjectNode();
        normalized.put("requestCount", capturedRequests.size());
        normalized.set("attempts", attempts);
        normalized.set("events", OBJECT_MAPPER.valueToTree(eventListener.events()));
        normalized.set(
                "variables",
                normalizeVariables(runtimeService.getVariables(processInstanceId), observeVariables));
        normalized.set("tasks", normalizeTasks(taskService, processInstanceId));
        normalized.putNull("error");
        return normalized;
    }

    private static ObjectNode executeJobAttempt(
            ManagementService managementService,
            String jobId,
            boolean normalizeNestedUnrecoverable) {

        RuntimeException executionException = null;
        try {
            managementService.executeJob(jobId);
        } catch (RuntimeException exception) {
            executionException = exception;
        }
        String executionError = executionException == null ? null : executionException.getMessage();

        Job executable = managementService.createJobQuery().jobId(jobId).singleResult();
        Job timer = managementService.createTimerJobQuery().jobId(jobId).singleResult();
        Job deadLetter = managementService.createDeadLetterJobQuery().jobId(jobId).singleResult();
        Job persisted = deadLetter != null ? deadLetter : timer != null ? timer : executable;
        ObjectNode attempt = OBJECT_MAPPER.createObjectNode();
        attempt.put("result", executionError == null ? "success" : "failure");
        if (executionError == null) {
            attempt.putNull("executionError");
        } else {
            attempt.put("executionError", executionError);
        }
        if (persisted == null) {
            attempt.put("jobState", "consumed");
            attempt.putNull("retries");
            attempt.putNull("errorMessage");
            attempt.put("dueDateSet", false);
        } else {
            attempt.put(
                    "jobState",
                    deadLetter != null ? "deadletter" : timer != null ? "timer" : "executable");
            attempt.put("retries", persisted.getRetries());
            if (persisted.getExceptionMessage() == null) {
                attempt.putNull("errorMessage");
            } else {
                attempt.put("errorMessage", persisted.getExceptionMessage());
            }
            attempt.put("dueDateSet", persisted.getDuedate() != null);
        }
        if (normalizeNestedUnrecoverable) {
            normalizeNestedUnrecoverableAttempt(
                    managementService,
                    jobId,
                    executionException,
                    persisted,
                    deadLetter != null,
                    timer != null,
                    attempt);
        }
        return attempt;
    }

    private static void normalizeNestedUnrecoverableAttempt(
            ManagementService managementService,
            String jobId,
            RuntimeException executionException,
            Job persisted,
            boolean deadLetter,
            boolean timer,
            ObjectNode attempt) {

        if (!(executionException instanceof FlowableException)
                || executionException instanceof FlowableUnrecoverableJobException) {
            throw new IllegalStateException(
                    "Nested unrecoverable contract must expose an outer FlowableException",
                    executionException);
        }
        FlowableUnrecoverableJobException unrecoverableCause =
                findUnrecoverableCause(executionException);
        if (unrecoverableCause == null) {
            throw new IllegalStateException(
                    "Nested unrecoverable contract did not preserve the typed cause",
                    executionException);
        }

        attempt.put("executionErrorKind", "generic");
        attempt.put("unrecoverableCauseMessage", unrecoverableCause.getMessage());
        String errorDetails = null;
        if (persisted != null) {
            if (deadLetter) {
                errorDetails = managementService.getDeadLetterJobExceptionStacktrace(jobId);
            } else if (timer) {
                errorDetails = managementService.getTimerJobExceptionStacktrace(jobId);
            } else {
                errorDetails = managementService.getJobExceptionStacktrace(jobId);
            }
        }
        attempt.put(
                "errorDetailsOuterMessagePresent",
                errorDetails != null && errorDetails.contains(executionException.getMessage()));
        attempt.put(
                "errorDetailsUnrecoverableCausePresent",
                errorDetails != null && errorDetails.contains(unrecoverableCause.getMessage()));
    }

    private static FlowableUnrecoverableJobException findUnrecoverableCause(Throwable throwable) {
        Throwable current = throwable;
        while (current != null) {
            if (current instanceof FlowableUnrecoverableJobException unrecoverable) {
                return unrecoverable;
            }
            current = current.getCause();
        }
        return null;
    }

    private static void handleRequest(
            HttpExchange exchange,
            JsonNode contractCase,
            List<CapturedRequest> capturedRequests,
            int attempt) throws IOException {

        byte[] requestBody;
        try (InputStream input = exchange.getRequestBody()) {
            requestBody = input.readAllBytes();
        }
        capturedRequests.add(new CapturedRequest(
                exchange.getRequestMethod(),
                exchange.getRequestURI().getPath(),
                new String(requestBody, StandardCharsets.UTF_8)));

        JsonNode response = responseForAttempt(contractCase, attempt);
        byte[] responseBody = OBJECT_MAPPER.writeValueAsBytes(response.path("body"));
        exchange.getResponseHeaders().set("Content-Type", "application/json");
        int status = response.path("status").asInt();
        exchange.sendResponseHeaders(status, responseBody.length);
        exchange.getResponseBody().write(responseBody);
        exchange.close();
    }

    private static JsonNode responseForAttempt(JsonNode contractCase, int attempt) {
        if (attempt == 0) {
            ObjectNode response = OBJECT_MAPPER.createObjectNode();
            // Support both top-level responseStatus/responseBody and httpStub status/body aliases.
            if (contractCase.has("responseStatus")) {
                response.set("status", contractCase.path("responseStatus"));
            } else {
                response.put("status", contractCase.path("status").asInt(200));
            }
            if (contractCase.has("responseBody")) {
                response.set("body", contractCase.path("responseBody"));
            } else if (contractCase.has("body")) {
                response.set("body", contractCase.path("body"));
            } else {
                response.set("body", OBJECT_MAPPER.createObjectNode());
            }
            return response;
        }
        JsonNode subsequent = contractCase.path("subsequentResponses");
        if (subsequent.isArray() && subsequent.size() >= attempt) {
            return subsequent.get(attempt - 1);
        }
        throw new IllegalStateException("No HTTP response configured for attempt " + (attempt + 1));
    }

    private static ObjectNode normalizeRequest(CapturedRequest request) {
        if (request == null) {
            throw new IllegalStateException("HTTP server did not receive the contract request");
        }
        ObjectNode normalized = OBJECT_MAPPER.createObjectNode();
        normalized.put("method", request.method());
        normalized.put("path", request.path());
        normalized.put("body", request.body());
        return normalized;
    }

    private static ObjectNode normalizeVariables(
            Map<String, Object> variables,
            List<String> observeVariables) throws IOException {
        ObjectNode normalized = OBJECT_MAPPER.createObjectNode();
        for (String name : observeVariables) {
            copyVariable(variables, normalized, name);
        }
        return normalized;
    }

    private static ArrayNode normalizeTasks(TaskService taskService, String processInstanceId) {
        List<Task> activeTasks = taskService.createTaskQuery()
                .processInstanceId(processInstanceId)
                .list();
        activeTasks.sort(Comparator.comparing(Task::getTaskDefinitionKey));
        ArrayNode tasks = OBJECT_MAPPER.createArrayNode();
        activeTasks.forEach(task -> tasks.add(task.getTaskDefinitionKey()));
        return tasks;
    }

    private static void copyVariable(
            Map<String, Object> variables,
            ObjectNode target,
            String name) throws IOException {

        if (variables.containsKey(name)) {
            Object value = variables.get(name);
            if ("responseBody".equals(name) && value != null && !(value instanceof String)) {
                // Flowable 8 uses its own Jackson generation internally. Parsing
                // the stable JSON representation avoids serializing JsonNode as
                // a Java bean when this runner uses a different Jackson package.
                target.set(name, OBJECT_MAPPER.readTree(value.toString()));
            } else {
                target.set(name, OBJECT_MAPPER.valueToTree(value));
            }
        }
    }

    private static final class GatedContractServer implements AutoCloseable {

        private final JsonNode contractCase;
        private final HttpServer server;
        private final String expectedPath;
        private final List<CapturedRequest> capturedRequests =
                Collections.synchronizedList(new ArrayList<>());
        private final AtomicInteger responseAttempt = new AtomicInteger();
        private final AtomicReference<Throwable> handlerFailure = new AtomicReference<>();
        private final CountDownLatch[] requestArrived = {
                new CountDownLatch(1),
                new CountDownLatch(1)
        };
        private final CountDownLatch[] allowResponse = {
                new CountDownLatch(1),
                new CountDownLatch(1)
        };

        private GatedContractServer(JsonNode contractCase) throws IOException {
            this.contractCase = contractCase;
            this.expectedPath = contractCase.path("path").asText();
            this.server = HttpServer.create(new InetSocketAddress("127.0.0.1", 0), 0);
            this.server.createContext(expectedPath, this::handleRequest);
            this.server.start();
        }

        private String endpoint() {
            return "http://127.0.0.1:" + server.getAddress().getPort() + expectedPath;
        }

        private void handleRequest(HttpExchange exchange) {
            int attempt = responseAttempt.getAndIncrement();
            boolean responseStarted = false;
            try {
                byte[] requestBody;
                try (InputStream input = exchange.getRequestBody()) {
                    requestBody = input.readAllBytes();
                }
                capturedRequests.add(new CapturedRequest(
                        exchange.getRequestMethod(),
                        exchange.getRequestURI().getPath(),
                        new String(requestBody, StandardCharsets.UTF_8)));
                if (attempt >= requestArrived.length) {
                    throw new IllegalStateException(
                            "Automatic retry contract received unexpected HTTP attempt "
                                    + (attempt + 1));
                }

                requestArrived[attempt].countDown();
                if (!allowResponse[attempt].await(WALL_TIMEOUT_MILLIS, TimeUnit.MILLISECONDS)) {
                    throw new IllegalStateException(
                            "Timed out waiting to release HTTP attempt " + (attempt + 1));
                }

                JsonNode response = responseForAttempt(contractCase, attempt);
                byte[] responseBody = OBJECT_MAPPER.writeValueAsBytes(response.path("body"));
                exchange.getResponseHeaders().set("Content-Type", "application/json");
                exchange.sendResponseHeaders(response.path("status").asInt(), responseBody.length);
                responseStarted = true;
                exchange.getResponseBody().write(responseBody);

            } catch (Throwable failure) {
                if (failure instanceof InterruptedException) {
                    Thread.currentThread().interrupt();
                }
                handlerFailure.compareAndSet(null, failure);
                if (!responseStarted) {
                    try {
                        byte[] body = "contract server failure".getBytes(StandardCharsets.UTF_8);
                        exchange.sendResponseHeaders(500, body.length);
                        exchange.getResponseBody().write(body);
                    } catch (IOException ignored) {
                        // The recorded handler failure is reported by the runner thread.
                    }
                }
            } finally {
                exchange.close();
            }
        }

        private void awaitRequest(int attempt) {
            try {
                if (!requestArrived[attempt].await(WALL_TIMEOUT_MILLIS, TimeUnit.MILLISECONDS)) {
                    throw new IllegalStateException(
                            "Timed out waiting for automatic HTTP attempt " + (attempt + 1));
                }
            } catch (InterruptedException exception) {
                Thread.currentThread().interrupt();
                throw new IllegalStateException(
                        "Interrupted while waiting for automatic HTTP attempt " + (attempt + 1),
                        exception);
            }
            throwIfHandlerFailed();
        }

        private void allowResponse(int attempt) {
            allowResponse[attempt].countDown();
        }

        private void releaseAllResponses() {
            for (CountDownLatch latch : allowResponse) {
                latch.countDown();
            }
        }

        private int requestCount() {
            return capturedRequests.size();
        }

        private void throwIfHandlerFailed() {
            Throwable failure = handlerFailure.get();
            if (failure != null) {
                throw new IllegalStateException("Automatic HTTP contract server failed", failure);
            }
        }

        @Override
        public void close() {
            server.stop(0);
        }
    }

    private record CapturedRequest(String method, String path, String body) {
    }
}
