pub const BPMN_NAMESPACE: &str = "http://www.omg.org/spec/BPMN/20100524/MODEL";
/// Name-stability alias retained for callers that still reference the older spelling; use `BPMN_NAMESPACE`.
#[allow(non_upper_case_globals)]
pub const Bpmn_NAMESPACE: &str = BPMN_NAMESPACE;
pub const XSI_NAMESPACE: &str = "http://www.w3.org/2001/XMLSchema-instance";
pub const XSD_NAMESPACE: &str = "http://www.w3.org/2001/XMLSchema";
pub const ACTIVITI_EXTENSIONS_NAMESPACE: &str = "http://activiti.org/bpmn";
pub const FLOWABLE_EXTENSIONS_NAMESPACE: &str = "http://flowable.org/bpmn";
pub const BPMN_DI_NAMESPACE: &str = "http://www.omg.org/spec/BPMN/20100524/DI";
/// Name-stability alias retained for callers that still reference the older spelling; use `BPMN_DI_NAMESPACE`.
#[allow(non_upper_case_globals)]
pub const BpmnDI_NAMESPACE: &str = BPMN_DI_NAMESPACE;
pub const OMGDC_NAMESPACE: &str = "http://www.omg.org/spec/DD/20100524/DC";
pub const OMGDI_NAMESPACE: &str = "http://www.omg.org/spec/DD/20100524/DI";

pub const ELEMENT_DEFINITIONS: &str = "definitions";
pub const ELEMENT_PROCESS: &str = "process";
pub const ELEMENT_DOCUMENTATION: &str = "documentation";
pub const ELEMENT_SUBPROCESS: &str = "subProcess";
pub const ELEMENT_TRANSACTION: &str = "transaction";
pub const ELEMENT_ADHOC_SUBPROCESS: &str = "adHocSubProcess";
pub const ELEMENT_EVENT_START: &str = "startEvent";
pub const ELEMENT_EVENT_END: &str = "endEvent";
pub const ELEMENT_EVENT_CATCH: &str = "intermediateCatchEvent";
pub const ELEMENT_EVENT_THROW: &str = "intermediateThrowEvent";
pub const ELEMENT_TASK_USER: &str = "userTask";
pub const ELEMENT_TASK: &str = "task";
pub const ELEMENT_TASK_SERVICE: &str = "serviceTask";
pub const ELEMENT_TASK_SEND: &str = "sendTask";
pub const ELEMENT_TASK_SCRIPT: &str = "scriptTask";
pub const ELEMENT_TASK_MANUAL: &str = "manualTask";
pub const ELEMENT_TASK_RECEIVE: &str = "receiveTask";
pub const ELEMENT_TASK_BUSINESS_RULE: &str = "businessRuleTask";
pub const ELEMENT_SEQUENCE_FLOW: &str = "sequenceFlow";
pub const ELEMENT_GATEWAY_EXCLUSIVE: &str = "exclusiveGateway";
pub const ELEMENT_GATEWAY_PARALLEL: &str = "parallelGateway";
pub const ELEMENT_GATEWAY_INCLUSIVE: &str = "inclusiveGateway";
pub const ELEMENT_GATEWAY_EVENT: &str = "eventBasedGateway";
pub const ELEMENT_GATEWAY_COMPLEX: &str = "complexGateway";
pub const ELEMENT_TEXT_ANNOTATION: &str = "textAnnotation";
pub const ELEMENT_GROUP: &str = "group";
pub const ELEMENT_CALL_ACTIVITY: &str = "callActivity";
pub const ELEMENT_DATA_OBJECT: &str = "dataObject";
pub const ELEMENT_DATA_OBJECT_REFERENCE: &str = "dataObjectReference";
pub const ELEMENT_DATA_VALUE: &str = "value";
pub const ELEMENT_EVENT_VARIABLELISTENERDEFINITION: &str = "variableListenerEventDefinition";
pub const ATTRIBUTE_VARIABLE_NAME: &str = "variableName";
pub const ATTRIBUTE_VARIABLE_CHANGE_TYPE: &str = "variableChangeType";
pub const ELEMENT_SOURCE_REF: &str = "sourceRef";
pub const ELEMENT_TARGET_REF: &str = "targetRef";
pub const ELEMENT_TRANSFORMATION: &str = "transformation";
pub const ELEMENT_ASSIGNMENT: &str = "assignment";
pub const ELEMENT_FROM: &str = "from";
pub const ELEMENT_TO: &str = "to";
pub const ELEMENT_FORMPROPERTY: &str = "formProperty";
pub const ELEMENT_EXECUTION_LISTENER: &str = "executionListener";
pub const ELEMENT_TASK_LISTENER: &str = "taskListener";
pub const ELEMENT_INPUT_ASSOCIATION: &str = "dataInputAssociation";
pub const ELEMENT_OUTPUT_ASSOCIATION: &str = "dataOutputAssociation";
pub const ELEMENT_IN_PARAMETERS: &str = "in";
pub const ELEMENT_OUT_PARAMETERS: &str = "out";
pub const ELEMENT_FIELD: &str = "field";
pub const ELEMENT_FIELD_STRING: &str = "string";
pub const ATTRIBUTE_FIELD_NAME: &str = "name";
pub const ATTRIBUTE_FIELD_STRING: &str = "stringValue";
pub const ATTRIBUTE_FIELD_EXPRESSION: &str = "expression";
pub const MAP_EXCEPTION: &str = "mapException";
pub const ELEMENT_FAILED_JOB_RETRY_TIME_CYCLE: &str = "failedJobRetryTimeCycle";
pub const ELEMENT_EVENT_BOUNDARY: &str = "boundaryEvent";
pub const ELEMENT_ERROR_EVENT_DEFINITION: &str = "errorEventDefinition";
pub const ELEMENT_ERROR: &str = "error";
pub const ELEMENT_MULTIINSTANCE: &str = "multiInstanceLoopCharacteristics";
pub const ELEMENT_MULTIINSTANCE_CARDINALITY: &str = "loopCardinality";
pub const ELEMENT_MULTIINSTANCE_DATAITEM: &str = "loopDataInputRef";
pub const ELEMENT_MULTIINSTANCE_CONDITION: &str = "completionCondition";
pub const ELEMENT_VARIABLE_AGGREGATION: &str = "variableAggregation";
pub const ELEMENT_VARIABLE: &str = "variable";

pub const ATTRIBUTE_MULTIINSTANCE_SEQUENTIAL: &str = "isSequential";
pub const ATTRIBUTE_MULTIINSTANCE_COLLECTION: &str = "collection";
pub const ATTRIBUTE_MULTIINSTANCE_ELEMENT_VARIABLE: &str = "elementVariable";
pub const ATTRIBUTE_MULTIINSTANCE_INDEX_VARIABLE: &str = "elementIndexVariable";
pub const ATTRIBUTE_MULTIINSTANCE_NO_WAIT_STATES_ASYNC_LEAVE: &str = "noWaitStatesAsyncLeave";
pub const ATTRIBUTE_CLASS: &str = "class";
pub const ATTRIBUTE_DELEGATEEXPRESSION: &str = "delegateExpression";

pub const ATTRIBUTE_ID: &str = "id";
pub const ATTRIBUTE_NAME: &str = "name";
pub const ATTRIBUTE_ITEM_SUBJECT_REF: &str = "itemSubjectRef";
pub const ATTRIBUTE_PROCESS_EXECUTABLE: &str = "isExecutable";
pub const ATTRIBUTE_ORDERING: &str = "ordering";
pub const ATTRIBUTE_CANCEL_REMAINING_INSTANCES: &str = "cancelRemainingInstances";
pub const ATTRIBUTE_EVENT_START_INITIATOR: &str = "initiator";
pub const ATTRIBUTE_FORM_FORMKEY: &str = "formKey";
pub const ATTRIBUTE_SAME_DEPLOYMENT: &str = "sameDeployment";
pub const ATTRIBUTE_TASK_USER_ASSIGNEE: &str = "assignee";
pub const ATTRIBUTE_TASK_USER_OWNER: &str = "owner";
pub const ATTRIBUTE_TASK_USER_CANDIDATE_USERS: &str = "candidateUsers";
pub const ATTRIBUTE_TASK_USER_CANDIDATE_GROUPS: &str = "candidateGroups";
pub const ATTRIBUTE_TASK_USER_PRIORITY: &str = "priority";
pub const ATTRIBUTE_TASK_USER_DUEDATE: &str = "dueDate";
pub const ATTRIBUTE_TASK_USER_BUSINESS_CALENDAR_NAME: &str = "businessCalendarName";
pub const ATTRIBUTE_TASK_USER_CATEGORY: &str = "category";
pub const ATTRIBUTE_TASK_SERVICE_EXTENSIONID: &str = "extensionId";
pub const ATTRIBUTE_TASK_USER_SKIP_EXPRESSION: &str = "skipExpression";
pub const ATTRIBUTE_TASK_ID_VARIABLE_NAME: &str = "idVariableName";
pub const ATTRIBUTE_TASK_COMPLETER_VARIABLE_NAME: &str = "taskCompleterVariableName";
pub const ATTRIBUTE_FLOW_SOURCE_REF: &str = "sourceRef";
pub const ATTRIBUTE_FLOW_TARGET_REF: &str = "targetRef";
pub const ATTRIBUTE_ACTIVITY_ASYNCHRONOUS: &str = "async";
pub const ATTRIBUTE_ACTIVITY_EXCLUSIVE: &str = "exclusive";
pub const ATTRIBUTE_ACTIVITY_ASYNCHRONOUS_LEAVE: &str = "asyncLeave";
pub const ATTRIBUTE_ACTIVITY_ASYNCHRONOUS_LEAVE_EXCLUSIVE: &str = "exclusiveLeave";
pub const ATTRIBUTE_CALL_ACTIVITY_CALLEDELEMENT: &str = "calledElement";
pub const ATTRIBUTE_CALL_ACTIVITY_CALLEDELEMENTTYPE: &str = "calledElementType";
pub const ATTRIBUTE_CALL_ACTIVITY_INHERITVARIABLES: &str = "inheritVariables";
pub const ATTRIBUTE_CALL_ACTIVITY_USE_LOCALSCOPE_FOR_OUTPARAMETERS: &str =
    "useLocalScopeForOutParameters";
pub const ATTRIBUTE_CALL_ACTIVITY_COMPLETE_ASYNC: &str = "completeAsync";
pub const ATTRIBUTE_FALLBACK_TO_DEFAULT_TENANT: &str = "fallbackToDefaultTenant";
pub const ATTRIBUTE_DEFAULT: &str = "default";
/// Java `BpmnXMLConstants.ATTRIBUTE_BUSINESS_KEY` — explicit business key expression
/// on call activity (`flowable:businessKey`).
pub const ATTRIBUTE_BUSINESS_KEY: &str = "businessKey";
/// Java `BpmnXMLConstants.ATTRIBUTE_INHERIT_BUSINESS_KEY`.
pub const ATTRIBUTE_INHERIT_BUSINESS_KEY: &str = "inheritBusinessKey";
/// Java `BpmnXMLConstants.ATTRIBUTE_CALL_ACTIVITY_PROCESS_INSTANCE_NAME`.
pub const ATTRIBUTE_CALL_ACTIVITY_PROCESS_INSTANCE_NAME: &str = "processInstanceName";
pub const ATTRIBUTE_DATA_ITEM_REF: &str = "itemSubjectRef";
pub const ATTRIBUTE_DATA_OBJECT_REF: &str = "dataObjectRef";
pub const ATTRIBUTE_IOPARAMETER_SOURCE: &str = "source";
pub const ATTRIBUTE_IOPARAMETER_SOURCE_EXPRESSION: &str = "sourceExpression";
pub const ATTRIBUTE_IOPARAMETER_TARGET: &str = "target";
pub const ATTRIBUTE_IOPARAMETER_TARGET_EXPRESSION: &str = "targetExpression";
pub const ATTRIBUTE_IOPARAMETER_TRANSIENT: &str = "transient";
pub const ATTRIBUTE_FORM_ID: &str = "id";
pub const ATTRIBUTE_FORM_NAME: &str = "name";
pub const ATTRIBUTE_FORM_TYPE: &str = "type";
pub const ATTRIBUTE_FORM_EXPRESSION: &str = "expression";
pub const ATTRIBUTE_FORM_VARIABLE: &str = "variable";
pub const ATTRIBUTE_FORM_DEFAULT: &str = "default";
pub const ATTRIBUTE_FORM_DATEPATTERN: &str = "datePattern";
pub const ATTRIBUTE_FORM_READABLE: &str = "readable";
/// Java `BpmnXMLConstants.ATTRIBUTE_FORM_WRITABLE`. The XML attribute is spelled
/// `writable`; only the model field is `writeable`. The writer already emits
/// `writable`, so a parser matching the model spelling would drop the flag on
/// every round-trip.
pub const ATTRIBUTE_FORM_WRITABLE: &str = "writable";
/// The model-side spelling, accepted on read for documents written by tools that
/// mirrored the field name instead of the Java attribute.
pub const ATTRIBUTE_FORM_WRITEABLE_ALIAS: &str = "writeable";
pub const ATTRIBUTE_FORM_REQUIRED: &str = "required";
pub const ATTRIBUTE_LISTENER_EVENT: &str = "event";
pub const ATTRIBUTE_LISTENER_CLASS: &str = "class";
pub const ATTRIBUTE_LISTENER_EXPRESSION: &str = "expression";
pub const ATTRIBUTE_LISTENER_DELEGATEEXPRESSION: &str = "delegateExpression";
pub const ATTRIBUTE_LISTENER_ON_TRANSACTION: &str = "onTransaction";
pub const ATTRIBUTE_ERROR_CODE: &str = "errorCode";
pub const MAP_EXCEPTION_ANDCHILDREN: &str = "andChildren";
pub const MAP_EXCEPTION_INCLUDECHILDEXCEPTIONS: &str = "includeChildExceptions";
pub const MAP_EXCEPTION_ROOTCAUSE: &str = "rootCause";
pub const ATTRIBUTE_TASK_SERVICE_CLASS: &str = "class";
pub const ATTRIBUTE_TASK_SERVICE_EXPRESSION: &str = "expression";
pub const ATTRIBUTE_TASK_SERVICE_DELEGATEEXPRESSION: &str = "delegateExpression";
pub const ATTRIBUTE_TASK_SERVICE_SKIP_EXPRESSION: &str = "skipExpression";
pub const ATTRIBUTE_TASK_RECEIVE_SKIP_EXPRESSION: &str = "skipExpression";
pub const ATTRIBUTE_TASK_SERVICE_USE_LOCAL_SCOPE_FOR_RESULT_VARIABLE: &str =
    "useLocalScopeForResultVariable";
pub const ATTRIBUTE_TASK_SERVICE_RESULT_VARIABLE_NAME: &str = "resultVariableName";
pub const ATTRIBUTE_TASK_SERVICE_STORE_RESULT_AS_TRANSIENT: &str = "storeResultVariableAsTransient";
pub const ATTRIBUTE_CALL_ACTIVITY_PROCESS_INSTANCE_ID_VARIABLE_NAME: &str =
    "processInstanceIdVariableName";
pub const ATTRIBUTE_BOUNDARY_ATTACHEDTOREF: &str = "attachedToRef";
pub const ATTRIBUTE_BOUNDARY_CANCELACTIVITY: &str = "cancelActivity";
pub const ATTRIBUTE_ERROR_REF: &str = "errorRef";
pub const ATTRIBUTE_ERROR_VARIABLE_NAME: &str = "errorVariableName";
pub const ATTRIBUTE_ERROR_VARIABLE_LOCAL_SCOPE: &str = "errorVariableLocalScope";
pub const ATTRIBUTE_ERROR_VARIABLE_TRANSIENT: &str = "errorVariableTransient";

pub const ATTRIBUTE_VALUE_TRUE: &str = "true";
pub const ATTRIBUTE_VALUE_FALSE: &str = "false";

pub const TARGET_NAMESPACE_ATTRIBUTE: &str = "targetNamespace";
pub const ATTRIBUTE_EXPORTER: &str = "exporter";
pub const ATTRIBUTE_EXPORTER_VERSION: &str = "exporterVersion";
pub const ATTRIBUTE_ACTIVITY_REF: &str = "activityRef";
pub const ATTRIBUTE_WAIT_FOR_COMPLETION: &str = "waitForCompletion";
pub const TYPE_LANGUAGE_ATTRIBUTE: &str = "typeLanguage";
pub const EXPRESSION_LANGUAGE_ATTRIBUTE: &str = "expressionLanguage";

pub const ELEMENT_DI_DIAGRAM: &str = "BPMNDiagram";
pub const ELEMENT_DI_PLANE: &str = "BPMNPlane";
pub const ELEMENT_DI_SHAPE: &str = "BPMNShape";
pub const ELEMENT_DI_EDGE: &str = "BPMNEdge";
pub const ELEMENT_DI_BOUNDS: &str = "Bounds";
pub const ELEMENT_DI_WAYPOINT: &str = "waypoint";
pub const ATTRIBUTE_DI_BPMNELEMENT: &str = "bpmnElement";
pub const ATTRIBUTE_DI_IS_EXPANDED: &str = "isExpanded";
pub const ATTRIBUTE_DI_X: &str = "x";
pub const ATTRIBUTE_DI_Y: &str = "y";
pub const ATTRIBUTE_DI_WIDTH: &str = "width";
pub const ATTRIBUTE_DI_HEIGHT: &str = "height";

pub const ELEMENT_COMPLETION_CONDITION: &str = "completionCondition";
pub const ELEMENT_FLOW_CONDITION: &str = "conditionExpression";
pub const ELEMENT_EVENT_TIMERDEFINITION: &str = "timerEventDefinition";
pub const ELEMENT_EVENT_MESSAGEDEFINITION: &str = "messageEventDefinition";
pub const ELEMENT_EVENT_SIGNALDEFINITION: &str = "signalEventDefinition";
pub const ELEMENT_EVENT_CANCELDEFINITION: &str = "cancelEventDefinition";
pub const ELEMENT_EVENT_COMPENSATEDEFINITION: &str = "compensateEventDefinition";
pub const ELEMENT_EVENT_CONDITIONALDEFINITION: &str = "conditionalEventDefinition";
pub const ELEMENT_EVENT_LINKDEFINITION: &str = "linkEventDefinition";
pub const ELEMENT_EVENT_ESCALATIONDEFINITION: &str = "escalationEventDefinition";
// Java BpmnXMLConstants:340-342
pub const ELEMENT_EVENT_TERMINATEDEFINITION: &str = "terminateEventDefinition";
pub const ATTRIBUTE_TERMINATE_ALL: &str = "terminateAll";
pub const ATTRIBUTE_TERMINATE_MULTI_INSTANCE: &str = "terminateMultiInstance";
pub const ELEMENT_ESCALATION: &str = "escalation";
pub const ATTRIBUTE_ESCALATION_REF: &str = "escalationRef";
pub const ATTRIBUTE_ESCALATION_CODE: &str = "escalationCode";
pub const ELEMENT_TIME_DATE: &str = "timeDate";
pub const ELEMENT_TIME_DURATION: &str = "timeDuration";
pub const ELEMENT_TIME_CYCLE: &str = "timeCycle";
