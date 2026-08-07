use crate::schema::manager::SchemaScript;

pub fn get_common_scripts() -> Vec<SchemaScript> {
    vec![
        // Property table (common)
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_GE_PROPERTY".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_GE_PROPERTY (
                    NAME_ TEXT PRIMARY KEY,
                    VALUE_ TEXT,
                    REV_ INTEGER DEFAULT 1
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_GE_PROPERTY".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_GE_PROPERTY (
                    NAME_ VARCHAR(255) PRIMARY KEY,
                    VALUE_ VARCHAR(300),
                    REV_ INTEGER DEFAULT 1
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_GE_PROPERTY".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_GE_PROPERTY (
                    NAME_ VARCHAR(255) PRIMARY KEY,
                    VALUE_ VARCHAR(300),
                    REV_ INTEGER DEFAULT 1
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#
            .to_string(),
        },
    ]
}

pub fn get_engine_scripts() -> Vec<SchemaScript> {
    vec![
        // Deployment table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RE_DEPLOYMENT".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RE_DEPLOYMENT (
                    ID_ TEXT PRIMARY KEY,
                    REV_ INTEGER DEFAULT 1,
                    NAME_ TEXT,
                    CATEGORY_ TEXT,
                    KEY_ TEXT,
                    TENANT_ID_ TEXT DEFAULT '',
                    DEPLOY_TIME_ INTEGER,
                    ENGINE_VERSION_ TEXT
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RE_DEPLOYMENT".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RE_DEPLOYMENT (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    REV_ INTEGER DEFAULT 1,
                    NAME_ VARCHAR(255),
                    CATEGORY_ VARCHAR(255),
                    KEY_ VARCHAR(255),
                    TENANT_ID_ VARCHAR(255) DEFAULT '',
                    DEPLOY_TIME_ BIGINT,
                    ENGINE_VERSION_ VARCHAR(255)
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RE_DEPLOYMENT".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RE_DEPLOYMENT (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    REV_ INTEGER DEFAULT 1,
                    NAME_ VARCHAR(255),
                    CATEGORY_ VARCHAR(255),
                    KEY_ VARCHAR(255),
                    TENANT_ID_ VARCHAR(255) DEFAULT '',
                    DEPLOY_TIME_ BIGINT,
                    ENGINE_VERSION_ VARCHAR(255)
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#
            .to_string(),
        },
        // Byte array table (deployment resources and general byte arrays)
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_GE_BYTEARRAY".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_GE_BYTEARRAY (
                    ID_ TEXT PRIMARY KEY,
                    REV_ INTEGER DEFAULT 1,
                    NAME_ TEXT,
                    DEPLOYMENT_ID_ TEXT,
                    BYTES_ BLOB,
                    GENERATED_ INTEGER DEFAULT 0
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_GE_BYTEARRAY".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_GE_BYTEARRAY (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    REV_ INTEGER DEFAULT 1,
                    NAME_ VARCHAR(255),
                    DEPLOYMENT_ID_ VARCHAR(255),
                    BYTES_ BYTEA,
                    GENERATED_ INTEGER DEFAULT 0
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_GE_BYTEARRAY".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_GE_BYTEARRAY (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    REV_ INTEGER DEFAULT 1,
                    NAME_ VARCHAR(255),
                    DEPLOYMENT_ID_ VARCHAR(255),
                    BYTES_ LONGBLOB,
                    GENERATED_ INTEGER DEFAULT 0
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#
            .to_string(),
        },
        // Process definition table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RE_PROCDEF".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RE_PROCDEF (
                    ID_ TEXT PRIMARY KEY,
                    REV_ INTEGER DEFAULT 1,
                    CATEGORY_ TEXT,
                    NAME_ TEXT,
                    KEY_ TEXT,
                    DESCRIPTION_ TEXT,
                    VERSION_ INTEGER,
                    RESOURCE_NAME_ TEXT,
                    DEPLOYMENT_ID_ TEXT,
                    DGRM_RESOURCE_NAME_ TEXT,
                    HAS_START_FORM_KEY_ INTEGER,
                    HAS_GRAPHICAL_NOTATION_ INTEGER,
                    SUSPENSION_STATE_ INTEGER,
                    TENANT_ID_ TEXT DEFAULT '',
                    ENGINE_VERSION_ TEXT,
                    APP_VERSION_ INTEGER
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RE_PROCDEF".to_string(),
            database_type: "postgres".to_string(),
            // ID_ is VARCHAR(255): engine builds ids as `{key}:{version}:{uuid}` which
            // exceeds classic Flowable VARCHAR(255) when keys are UUID-suffixed.
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RE_PROCDEF (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    REV_ INTEGER DEFAULT 1,
                    CATEGORY_ VARCHAR(255),
                    NAME_ VARCHAR(255),
                    KEY_ VARCHAR(255),
                    DESCRIPTION_ TEXT,
                    VERSION_ INTEGER,
                    RESOURCE_NAME_ VARCHAR(4000),
                    DEPLOYMENT_ID_ VARCHAR(255),
                    DGRM_RESOURCE_NAME_ VARCHAR(4000),
                    HAS_START_FORM_KEY_ INTEGER,
                    HAS_GRAPHICAL_NOTATION_ INTEGER,
                    SUSPENSION_STATE_ INTEGER,
                    TENANT_ID_ VARCHAR(255) DEFAULT '',
                    ENGINE_VERSION_ VARCHAR(255),
                    APP_VERSION_ INTEGER
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RE_PROCDEF".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RE_PROCDEF (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    REV_ INTEGER DEFAULT 1,
                    CATEGORY_ VARCHAR(255),
                    NAME_ VARCHAR(255),
                    KEY_ VARCHAR(255),
                    DESCRIPTION_ TEXT,
                    VERSION_ INTEGER,
                    RESOURCE_NAME_ VARCHAR(4000),
                    DEPLOYMENT_ID_ VARCHAR(255),
                    DGRM_RESOURCE_NAME_ VARCHAR(4000),
                    HAS_START_FORM_KEY_ INTEGER,
                    HAS_GRAPHICAL_NOTATION_ INTEGER,
                    SUSPENSION_STATE_ INTEGER,
                    TENANT_ID_ VARCHAR(255) DEFAULT '',
                    ENGINE_VERSION_ VARCHAR(255),
                    APP_VERSION_ INTEGER
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#
            .to_string(),
        },
        // Execution table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RU_EXECUTION".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RU_EXECUTION (
                    ID_ TEXT PRIMARY KEY,
                    REV_ INTEGER DEFAULT 1,
                    PROC_INST_ID_ TEXT,
                    BUSINESS_KEY_ TEXT,
                    PARENT_ID_ TEXT,
                    PROC_DEF_ID_ TEXT,
                    SUPER_EXEC_ TEXT,
                    ROOT_PROC_INST_ID_ TEXT,
                    ACT_ID_ TEXT,
                    IS_ACTIVE_ INTEGER,
                    IS_CONCURRENT_ INTEGER,
                    IS_SCOPE_ INTEGER,
                    IS_EVENT_SCOPE_ INTEGER,
                    IS_MI_ROOT_ INTEGER,
                    SUSPENSION_STATE_ INTEGER DEFAULT 1,
                    CACHED_ENT_STATE_ INTEGER,
                    TENANT_ID_ TEXT DEFAULT '',
                    NAME_ TEXT,
                    START_ACT_ID_ TEXT,
                    START_TIME_ INTEGER,
                    START_USER_ID_ TEXT,
                    LOCK_TIME_ INTEGER,
                    IS_COUNT_ENABLED_ INTEGER,
                    EVT_SUBSCR_COUNT_ INTEGER,
                    TASK_COUNT_ INTEGER,
                    JOB_COUNT_ INTEGER,
                    TIMER_JOB_COUNT_ INTEGER,
                    SUSP_JOB_COUNT_ INTEGER,
                    DEADLETTER_JOB_COUNT_ INTEGER,
                    VAR_COUNT_ INTEGER,
                    ID_LINK_COUNT_ INTEGER
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RU_EXECUTION".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RU_EXECUTION (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    REV_ INTEGER DEFAULT 1,
                    PROC_INST_ID_ VARCHAR(255),
                    BUSINESS_KEY_ VARCHAR(255),
                    PARENT_ID_ VARCHAR(255),
                    PROC_DEF_ID_ VARCHAR(255),
                    SUPER_EXEC_ VARCHAR(255),
                    ROOT_PROC_INST_ID_ VARCHAR(255),
                    ACT_ID_ VARCHAR(255),
                    IS_ACTIVE_ BOOLEAN,
                    IS_CONCURRENT_ BOOLEAN,
                    IS_SCOPE_ BOOLEAN,
                    IS_EVENT_SCOPE_ BOOLEAN,
                    IS_MI_ROOT_ BOOLEAN,
                    SUSPENSION_STATE_ INTEGER DEFAULT 1,
                    CACHED_ENT_STATE_ INTEGER,
                    TENANT_ID_ VARCHAR(255) DEFAULT '',
                    NAME_ VARCHAR(255),
                    START_ACT_ID_ VARCHAR(255),
                    START_TIME_ BIGINT,
                    START_USER_ID_ VARCHAR(255),
                    LOCK_TIME_ BIGINT,
                    IS_COUNT_ENABLED_ BOOLEAN,
                    EVT_SUBSCR_COUNT_ INTEGER,
                    TASK_COUNT_ INTEGER,
                    JOB_COUNT_ INTEGER,
                    TIMER_JOB_COUNT_ INTEGER,
                    SUSP_JOB_COUNT_ INTEGER,
                    DEADLETTER_JOB_COUNT_ INTEGER,
                    VAR_COUNT_ INTEGER,
                    ID_LINK_COUNT_ INTEGER
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RU_EXECUTION".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RU_EXECUTION (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    REV_ INTEGER DEFAULT 1,
                    PROC_INST_ID_ VARCHAR(255),
                    BUSINESS_KEY_ VARCHAR(255),
                    PARENT_ID_ VARCHAR(255),
                    PROC_DEF_ID_ VARCHAR(255),
                    SUPER_EXEC_ VARCHAR(255),
                    ROOT_PROC_INST_ID_ VARCHAR(255),
                    ACT_ID_ VARCHAR(255),
                    IS_ACTIVE_ BOOLEAN,
                    IS_CONCURRENT_ BOOLEAN,
                    IS_SCOPE_ BOOLEAN,
                    IS_EVENT_SCOPE_ BOOLEAN,
                    IS_MI_ROOT_ BOOLEAN,
                    SUSPENSION_STATE_ INTEGER DEFAULT 1,
                    CACHED_ENT_STATE_ INTEGER,
                    TENANT_ID_ VARCHAR(255) DEFAULT '',
                    NAME_ VARCHAR(255),
                    START_ACT_ID_ VARCHAR(255),
                    START_TIME_ BIGINT,
                    START_USER_ID_ VARCHAR(255),
                    LOCK_TIME_ BIGINT,
                    IS_COUNT_ENABLED_ BOOLEAN,
                    EVT_SUBSCR_COUNT_ INTEGER,
                    TASK_COUNT_ INTEGER,
                    JOB_COUNT_ INTEGER,
                    TIMER_JOB_COUNT_ INTEGER,
                    SUSP_JOB_COUNT_ INTEGER,
                    DEADLETTER_JOB_COUNT_ INTEGER,
                    VAR_COUNT_ INTEGER,
                    ID_LINK_COUNT_ INTEGER
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#
            .to_string(),
        },
        // Task table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RU_TASK".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RU_TASK (
                    ID_ TEXT PRIMARY KEY,
                    REV_ INTEGER DEFAULT 1,
                    EXECUTION_ID_ TEXT,
                    PROC_INST_ID_ TEXT,
                    PROC_DEF_ID_ TEXT,
                    NAME_ TEXT,
                    BUSINESS_KEY_ TEXT,
                    PARENT_TASK_ID_ TEXT,
                    DESCRIPTION_ TEXT,
                    TASK_DEF_KEY_ TEXT,
                    OWNER_ TEXT,
                    ASSIGNEE_ TEXT,
                    DELEGATION_ TEXT,
                    PRIORITY_ INTEGER,
                    CREATE_TIME_ INTEGER,
                    DUE_DATE_ INTEGER,
                    CATEGORY_ TEXT,
                    SUSPENSION_STATE_ INTEGER DEFAULT 1,
                    TENANT_ID_ TEXT DEFAULT '',
                    FORM_KEY_ TEXT,
                    CLAIM_TIME_ INTEGER,
                    APP_VERSION_ INTEGER
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RU_TASK".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RU_TASK (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    REV_ INTEGER DEFAULT 1,
                    EXECUTION_ID_ VARCHAR(255),
                    PROC_INST_ID_ VARCHAR(255),
                    PROC_DEF_ID_ VARCHAR(255),
                    NAME_ VARCHAR(255),
                    BUSINESS_KEY_ VARCHAR(255),
                    PARENT_TASK_ID_ VARCHAR(255),
                    DESCRIPTION_ TEXT,
                    TASK_DEF_KEY_ VARCHAR(255),
                    OWNER_ VARCHAR(255),
                    ASSIGNEE_ VARCHAR(255),
                    DELEGATION_ VARCHAR(255),
                    PRIORITY_ INTEGER,
                    CREATE_TIME_ BIGINT,
                    DUE_DATE_ BIGINT,
                    CATEGORY_ VARCHAR(255),
                    SUSPENSION_STATE_ INTEGER DEFAULT 1,
                    TENANT_ID_ VARCHAR(255) DEFAULT '',
                    FORM_KEY_ VARCHAR(255),
                    CLAIM_TIME_ BIGINT,
                    APP_VERSION_ INTEGER
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RU_TASK".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RU_TASK (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    REV_ INTEGER DEFAULT 1,
                    EXECUTION_ID_ VARCHAR(255),
                    PROC_INST_ID_ VARCHAR(255),
                    PROC_DEF_ID_ VARCHAR(255),
                    NAME_ VARCHAR(255),
                    BUSINESS_KEY_ VARCHAR(255),
                    PARENT_TASK_ID_ VARCHAR(255),
                    DESCRIPTION_ TEXT,
                    TASK_DEF_KEY_ VARCHAR(255),
                    OWNER_ VARCHAR(255),
                    ASSIGNEE_ VARCHAR(255),
                    DELEGATION_ VARCHAR(255),
                    PRIORITY_ INTEGER,
                    CREATE_TIME_ BIGINT,
                    DUE_DATE_ BIGINT,
                    CATEGORY_ VARCHAR(255),
                    SUSPENSION_STATE_ INTEGER DEFAULT 1,
                    TENANT_ID_ VARCHAR(255) DEFAULT '',
                    FORM_KEY_ VARCHAR(255),
                    CLAIM_TIME_ BIGINT,
                    APP_VERSION_ INTEGER
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#
            .to_string(),
        },
        // Variable table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RU_VARIABLE".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RU_VARIABLE (
                    ID_ TEXT PRIMARY KEY,
                    REV_ INTEGER DEFAULT 1,
                    TYPE_ TEXT,
                    NAME_ TEXT,
                    EXECUTION_ID_ TEXT,
                    PROC_INST_ID_ TEXT,
                    TASK_ID_ TEXT,
                    SCOPE_TYPE_ TEXT,
                    SCOPE_ID_ TEXT,
                    SUB_SCOPE_ID_ TEXT,
                    BYTEARRAY_ID_ TEXT,
                    DOUBLE_ REAL,
                    LONG_ INTEGER,
                    TEXT_ TEXT,
                    TEXT2_ TEXT,
                    IS_INITIAL_ INTEGER
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RU_VARIABLE".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RU_VARIABLE (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    REV_ INTEGER DEFAULT 1,
                    TYPE_ VARCHAR(255),
                    NAME_ VARCHAR(255),
                    EXECUTION_ID_ VARCHAR(255),
                    PROC_INST_ID_ VARCHAR(255),
                    TASK_ID_ VARCHAR(255),
                    SCOPE_TYPE_ VARCHAR(255),
                    SCOPE_ID_ VARCHAR(255),
                    SUB_SCOPE_ID_ VARCHAR(255),
                    BYTEARRAY_ID_ VARCHAR(255),
                    DOUBLE_ DOUBLE PRECISION,
                    LONG_ BIGINT,
                    TEXT_ TEXT,
                    TEXT2_ TEXT,
                    IS_INITIAL_ BOOLEAN
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RU_VARIABLE".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RU_VARIABLE (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    REV_ INTEGER DEFAULT 1,
                    TYPE_ VARCHAR(255),
                    NAME_ VARCHAR(255),
                    EXECUTION_ID_ VARCHAR(255),
                    PROC_INST_ID_ VARCHAR(255),
                    TASK_ID_ VARCHAR(255),
                    SCOPE_TYPE_ VARCHAR(255),
                    SCOPE_ID_ VARCHAR(255),
                    SUB_SCOPE_ID_ VARCHAR(255),
                    BYTEARRAY_ID_ VARCHAR(255),
                    DOUBLE_ DOUBLE,
                    LONG_ BIGINT,
                    TEXT_ TEXT,
                    TEXT2_ TEXT,
                    IS_INITIAL_ BOOLEAN
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#
            .to_string(),
        },
        // Job table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RU_JOB".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RU_JOB (
                    ID_ TEXT PRIMARY KEY,
                    REV_ INTEGER DEFAULT 1,
                    TYPE_ TEXT,
                    PROC_DEF_ID_ TEXT,
                    PROC_INST_ID_ TEXT,
                    EXECUTION_ID_ TEXT,
                    NAME_ TEXT,
                    SCOPE_TYPE_ TEXT,
                    SCOPE_ID_ TEXT,
                    SUB_SCOPE_ID_ TEXT,
                    CREATE_TIME_ INTEGER,
                    LOCK_OWNER_ TEXT,
                    LOCK_TIME_ INTEGER,
                    EXCLUSIVE_ INTEGER,
                    EXECUTION_ TEXT,
                    PROCESS_DEFINITION_ TEXT,
                    RETRIES_ INTEGER,
                    EXCEPTION_STACK_ID_ TEXT,
                    EXCEPTION_MSG_ TEXT,
                    DUEDATE_ INTEGER,
                    REPEAT_ TEXT,
                    HISTORY_URL_ TEXT,
                    HANDLER_TYPE_ TEXT,
                    CUSTOM_VALUES_ID_ TEXT
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RU_JOB".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RU_JOB (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    REV_ INTEGER DEFAULT 1,
                    TYPE_ VARCHAR(255),
                    PROC_DEF_ID_ VARCHAR(255),
                    PROC_INST_ID_ VARCHAR(255),
                    EXECUTION_ID_ VARCHAR(255),
                    NAME_ VARCHAR(255),
                    SCOPE_TYPE_ VARCHAR(255),
                    SCOPE_ID_ VARCHAR(255),
                    SUB_SCOPE_ID_ VARCHAR(255),
                    CREATE_TIME_ BIGINT,
                    LOCK_OWNER_ VARCHAR(255),
                    LOCK_TIME_ BIGINT,
                    EXCLUSIVE_ BOOLEAN,
                    EXECUTION_ VARCHAR(255),
                    PROCESS_DEFINITION_ VARCHAR(255),
                    RETRIES_ INTEGER,
                    EXCEPTION_STACK_ID_ VARCHAR(255),
                    EXCEPTION_MSG_ TEXT,
                    DUEDATE_ BIGINT,
                    REPEAT_ VARCHAR(255),
                    HISTORY_URL_ VARCHAR(255),
                    HANDLER_TYPE_ VARCHAR(255),
                    CUSTOM_VALUES_ID_ VARCHAR(255)
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RU_JOB".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RU_JOB (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    REV_ INTEGER DEFAULT 1,
                    TYPE_ VARCHAR(255),
                    PROC_DEF_ID_ VARCHAR(255),
                    PROC_INST_ID_ VARCHAR(255),
                    EXECUTION_ID_ VARCHAR(255),
                    NAME_ VARCHAR(255),
                    SCOPE_TYPE_ VARCHAR(255),
                    SCOPE_ID_ VARCHAR(255),
                    SUB_SCOPE_ID_ VARCHAR(255),
                    CREATE_TIME_ BIGINT,
                    LOCK_OWNER_ VARCHAR(255),
                    LOCK_TIME_ BIGINT,
                    EXCLUSIVE_ BOOLEAN,
                    EXECUTION_ VARCHAR(255),
                    PROCESS_DEFINITION_ VARCHAR(255),
                    RETRIES_ INTEGER,
                    EXCEPTION_STACK_ID_ VARCHAR(255),
                    EXCEPTION_MSG_ TEXT,
                    DUEDATE_ BIGINT,
                    REPEAT_ VARCHAR(255),
                    HISTORY_URL_ VARCHAR(255),
                    HANDLER_TYPE_ VARCHAR(255),
                    CUSTOM_VALUES_ID_ VARCHAR(255)
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#
            .to_string(),
        },
        // History process instance table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_HI_PROCINST".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_HI_PROCINST (
                    ID_ TEXT PRIMARY KEY,
                    REV_ INTEGER DEFAULT 1,
                    PROC_DEF_ID_ TEXT,
                    PROC_DEF_KEY_ TEXT,
                    PROC_DEF_NAME_ TEXT,
                    PROC_DEF_VERSION_ INTEGER,
                    BUSINESS_KEY_ TEXT,
                    START_TIME_ INTEGER,
                    END_TIME_ INTEGER,
                    DURATION_ INTEGER,
                    START_USER_ID_ TEXT,
                    START_ACT_ID_ TEXT,
                    END_ACT_ID_ TEXT,
                    SUPER_PROCESS_INSTANCE_ID_ TEXT,
                    DELETE_REASON_ TEXT,
                    TENANT_ID_ TEXT DEFAULT '',
                    NAME_ TEXT,
                    DESCRIPTION_ TEXT,
                    CALLBACK_ID_ TEXT,
                    CALLBACK_TYPE_ TEXT,
                    REFERENCE_ID_ TEXT,
                    REFERENCE_TYPE_ TEXT
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_HI_PROCINST".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_HI_PROCINST (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    REV_ INTEGER DEFAULT 1,
                    PROC_DEF_ID_ VARCHAR(255),
                    PROC_DEF_KEY_ VARCHAR(255),
                    PROC_DEF_NAME_ VARCHAR(255),
                    PROC_DEF_VERSION_ INTEGER,
                    BUSINESS_KEY_ VARCHAR(255),
                    START_TIME_ BIGINT,
                    END_TIME_ BIGINT,
                    DURATION_ BIGINT,
                    START_USER_ID_ VARCHAR(255),
                    START_ACT_ID_ VARCHAR(255),
                    END_ACT_ID_ VARCHAR(255),
                    SUPER_PROCESS_INSTANCE_ID_ VARCHAR(255),
                    DELETE_REASON_ VARCHAR(4000),
                    TENANT_ID_ VARCHAR(255) DEFAULT '',
                    NAME_ VARCHAR(255),
                    DESCRIPTION_ VARCHAR(4000),
                    CALLBACK_ID_ VARCHAR(255),
                    CALLBACK_TYPE_ VARCHAR(255),
                    REFERENCE_ID_ VARCHAR(255),
                    REFERENCE_TYPE_ VARCHAR(255)
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_HI_PROCINST".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_HI_PROCINST (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    REV_ INTEGER DEFAULT 1,
                    PROC_DEF_ID_ VARCHAR(255),
                    PROC_DEF_KEY_ VARCHAR(255),
                    PROC_DEF_NAME_ VARCHAR(255),
                    PROC_DEF_VERSION_ INTEGER,
                    BUSINESS_KEY_ VARCHAR(255),
                    START_TIME_ BIGINT,
                    END_TIME_ BIGINT,
                    DURATION_ BIGINT,
                    START_USER_ID_ VARCHAR(255),
                    START_ACT_ID_ VARCHAR(255),
                    END_ACT_ID_ VARCHAR(255),
                    SUPER_PROCESS_INSTANCE_ID_ VARCHAR(255),
                    DELETE_REASON_ VARCHAR(4000),
                    TENANT_ID_ VARCHAR(255) DEFAULT '',
                    NAME_ VARCHAR(255),
                    DESCRIPTION_ VARCHAR(4000),
                    CALLBACK_ID_ VARCHAR(255),
                    CALLBACK_TYPE_ VARCHAR(255),
                    REFERENCE_ID_ VARCHAR(255),
                    REFERENCE_TYPE_ VARCHAR(255)
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#
            .to_string(),
        },
        // History task instance table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_HI_TASKINST".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_HI_TASKINST (
                    ID_ TEXT PRIMARY KEY,
                    REV_ INTEGER DEFAULT 1,
                    PROC_DEF_ID_ TEXT,
                    PROC_INST_ID_ TEXT,
                    EXECUTION_ID_ TEXT,
                    NAME_ TEXT,
                    PARENT_TASK_ID_ TEXT,
                    DESCRIPTION_ TEXT,
                    OWNER_ TEXT,
                    ASSIGNEE_ TEXT,
                    START_TIME_ INTEGER,
                    CLAIM_TIME_ INTEGER,
                    END_TIME_ INTEGER,
                    DURATION_ INTEGER,
                    DELETE_REASON_ TEXT,
                    PRIORITY_ INTEGER,
                    DUE_DATE_ INTEGER,
                    TASK_DEF_KEY_ TEXT,
                    CATEGORY_ TEXT,
                    FORM_KEY_ TEXT,
                    TENANT_ID_ TEXT DEFAULT '',
                    APP_VERSION_ INTEGER
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_HI_TASKINST".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_HI_TASKINST (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    REV_ INTEGER DEFAULT 1,
                    PROC_DEF_ID_ VARCHAR(255),
                    PROC_INST_ID_ VARCHAR(255),
                    EXECUTION_ID_ VARCHAR(255),
                    NAME_ VARCHAR(255),
                    PARENT_TASK_ID_ VARCHAR(255),
                    DESCRIPTION_ VARCHAR(4000),
                    OWNER_ VARCHAR(255),
                    ASSIGNEE_ VARCHAR(255),
                    START_TIME_ BIGINT,
                    CLAIM_TIME_ BIGINT,
                    END_TIME_ BIGINT,
                    DURATION_ BIGINT,
                    DELETE_REASON_ VARCHAR(4000),
                    PRIORITY_ INTEGER,
                    DUE_DATE_ BIGINT,
                    TASK_DEF_KEY_ VARCHAR(255),
                    CATEGORY_ VARCHAR(255),
                    FORM_KEY_ VARCHAR(255),
                    TENANT_ID_ VARCHAR(255) DEFAULT '',
                    APP_VERSION_ INTEGER
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_HI_TASKINST".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_HI_TASKINST (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    REV_ INTEGER DEFAULT 1,
                    PROC_DEF_ID_ VARCHAR(255),
                    PROC_INST_ID_ VARCHAR(255),
                    EXECUTION_ID_ VARCHAR(255),
                    NAME_ VARCHAR(255),
                    PARENT_TASK_ID_ VARCHAR(255),
                    DESCRIPTION_ VARCHAR(4000),
                    OWNER_ VARCHAR(255),
                    ASSIGNEE_ VARCHAR(255),
                    START_TIME_ BIGINT,
                    CLAIM_TIME_ BIGINT,
                    END_TIME_ BIGINT,
                    DURATION_ BIGINT,
                    DELETE_REASON_ VARCHAR(4000),
                    PRIORITY_ INTEGER,
                    DUE_DATE_ BIGINT,
                    TASK_DEF_KEY_ VARCHAR(255),
                    CATEGORY_ VARCHAR(255),
                    FORM_KEY_ VARCHAR(255),
                    TENANT_ID_ VARCHAR(255) DEFAULT '',
                    APP_VERSION_ INTEGER
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#
            .to_string(),
        },
        // History variable instance table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_HI_VARINST".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_HI_VARINST (
                    ID_ TEXT PRIMARY KEY,
                    REV_ INTEGER DEFAULT 1,
                    PROC_INST_ID_ TEXT,
                    EXECUTION_ID_ TEXT,
                    TASK_ID_ TEXT,
                    CREATE_TIME_ INTEGER,
                    LAST_UPDATED_TIME_ INTEGER,
                    NAME_ TEXT,
                    VAR_TYPE_ TEXT,
                    SCOPE_TYPE_ TEXT,
                    SCOPE_ID_ TEXT,
                    SUB_SCOPE_ID_ TEXT,
                    BYTEARRAY_ID_ TEXT,
                    DOUBLE_ REAL,
                    LONG_ INTEGER,
                    TEXT_ TEXT,
                    TEXT2_ TEXT
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_HI_VARINST".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_HI_VARINST (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    REV_ INTEGER DEFAULT 1,
                    PROC_INST_ID_ VARCHAR(255),
                    EXECUTION_ID_ VARCHAR(255),
                    TASK_ID_ VARCHAR(255),
                    CREATE_TIME_ BIGINT,
                    LAST_UPDATED_TIME_ BIGINT,
                    NAME_ VARCHAR(255),
                    VAR_TYPE_ VARCHAR(255),
                    SCOPE_TYPE_ VARCHAR(255),
                    SCOPE_ID_ VARCHAR(255),
                    SUB_SCOPE_ID_ VARCHAR(255),
                    BYTEARRAY_ID_ VARCHAR(255),
                    DOUBLE_ DOUBLE PRECISION,
                    LONG_ BIGINT,
                    TEXT_ TEXT,
                    TEXT2_ TEXT
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_HI_VARINST".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_HI_VARINST (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    REV_ INTEGER DEFAULT 1,
                    PROC_INST_ID_ VARCHAR(255),
                    EXECUTION_ID_ VARCHAR(255),
                    TASK_ID_ VARCHAR(255),
                    CREATE_TIME_ BIGINT,
                    LAST_UPDATED_TIME_ BIGINT,
                    NAME_ VARCHAR(255),
                    VAR_TYPE_ VARCHAR(255),
                    SCOPE_TYPE_ VARCHAR(255),
                    SCOPE_ID_ VARCHAR(255),
                    SUB_SCOPE_ID_ VARCHAR(255),
                    BYTEARRAY_ID_ VARCHAR(255),
                    DOUBLE_ DOUBLE,
                    LONG_ BIGINT,
                    TEXT_ TEXT,
                    TEXT2_ TEXT
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#
            .to_string(),
        },
        // Event Registry Deployment table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_EVT_DEPLOYMENT".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_EVT_DEPLOYMENT (
                    ID_ TEXT PRIMARY KEY,
                    NAME_ TEXT,
                    DEPLOYED_AT_ INTEGER,
                    CATEGORY_ TEXT,
                    PARENT_DEPLOYMENT_ID_ TEXT,
                    TENANT_ID_ TEXT,
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_EVT_DEPLOYMENT".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_EVT_DEPLOYMENT (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    NAME_ VARCHAR(255),
                    DEPLOYED_AT_ BIGINT,
                    CATEGORY_ VARCHAR(255),
                    PARENT_DEPLOYMENT_ID_ VARCHAR(255),
                    TENANT_ID_ VARCHAR(255),
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_EVT_DEPLOYMENT".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_EVT_DEPLOYMENT (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    NAME_ VARCHAR(255),
                    DEPLOYED_AT_ BIGINT,
                    CATEGORY_ VARCHAR(255),
                    PARENT_DEPLOYMENT_ID_ VARCHAR(255),
                    TENANT_ID_ VARCHAR(255),
                    DATA_ TEXT NOT NULL
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#
            .to_string(),
        },
        // Event Registry Channel Definition table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_EVT_CHANNEL_DEF".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_EVT_CHANNEL_DEF (
                    ID_ TEXT PRIMARY KEY,
                    DEPLOYMENT_ID_ TEXT,
                    KEY_ TEXT,
                    NAME_ TEXT,
                    DESCRIPTION_ TEXT,
                    CATEGORY_ TEXT,
                    CHANNEL_TYPE_ TEXT,
                    RESOURCE_NAME_ TEXT,
                    VERSION_ INTEGER,
                    CREATE_TIME_ INTEGER,
                    TENANT_ID_ TEXT,
                    PARENT_DEPLOYMENT_ID_ TEXT,
                    CONFIGURATION_ TEXT,
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_EVT_CHANNEL_DEF".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_EVT_CHANNEL_DEF (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    DEPLOYMENT_ID_ VARCHAR(255),
                    KEY_ VARCHAR(255),
                    NAME_ VARCHAR(255),
                    DESCRIPTION_ TEXT,
                    CATEGORY_ VARCHAR(255),
                    CHANNEL_TYPE_ VARCHAR(255),
                    RESOURCE_NAME_ VARCHAR(255),
                    VERSION_ INTEGER,
                    CREATE_TIME_ BIGINT,
                    TENANT_ID_ VARCHAR(255),
                    PARENT_DEPLOYMENT_ID_ VARCHAR(255),
                    CONFIGURATION_ TEXT,
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_EVT_CHANNEL_DEF".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_EVT_CHANNEL_DEF (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    DEPLOYMENT_ID_ VARCHAR(255),
                    KEY_ VARCHAR(255),
                    NAME_ VARCHAR(255),
                    DESCRIPTION_ TEXT,
                    CATEGORY_ VARCHAR(255),
                    CHANNEL_TYPE_ VARCHAR(255),
                    RESOURCE_NAME_ VARCHAR(255),
                    VERSION_ INTEGER,
                    CREATE_TIME_ BIGINT,
                    TENANT_ID_ VARCHAR(255),
                    PARENT_DEPLOYMENT_ID_ VARCHAR(255),
                    CONFIGURATION_ TEXT,
                    DATA_ TEXT NOT NULL
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#
            .to_string(),
        },
        // Event Registry Event Definition table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_EVT_EVENT_DEF".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_EVT_EVENT_DEF (
                    ID_ TEXT PRIMARY KEY,
                    DEPLOYMENT_ID_ TEXT,
                    KEY_ TEXT,
                    NAME_ TEXT,
                    DESCRIPTION_ TEXT,
                    CATEGORY_ TEXT,
                    EVENT_TYPE_ TEXT,
                    CHANNEL_KEY_ TEXT,
                    RESOURCE_NAME_ TEXT,
                    VERSION_ INTEGER,
                    CREATE_TIME_ INTEGER,
                    TENANT_ID_ TEXT,
                    PARENT_DEPLOYMENT_ID_ TEXT,
                    PAYLOAD_ TEXT,
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_EVT_EVENT_DEF".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_EVT_EVENT_DEF (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    DEPLOYMENT_ID_ VARCHAR(255),
                    KEY_ VARCHAR(255),
                    NAME_ VARCHAR(255),
                    DESCRIPTION_ TEXT,
                    CATEGORY_ VARCHAR(255),
                    EVENT_TYPE_ VARCHAR(255),
                    CHANNEL_KEY_ VARCHAR(255),
                    RESOURCE_NAME_ VARCHAR(255),
                    VERSION_ INTEGER,
                    CREATE_TIME_ BIGINT,
                    TENANT_ID_ VARCHAR(255),
                    PARENT_DEPLOYMENT_ID_ VARCHAR(255),
                    PAYLOAD_ TEXT,
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_EVT_EVENT_DEF".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_EVT_EVENT_DEF (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    DEPLOYMENT_ID_ VARCHAR(255),
                    KEY_ VARCHAR(255),
                    NAME_ VARCHAR(255),
                    DESCRIPTION_ TEXT,
                    CATEGORY_ VARCHAR(255),
                    EVENT_TYPE_ VARCHAR(255),
                    CHANNEL_KEY_ VARCHAR(255),
                    RESOURCE_NAME_ VARCHAR(255),
                    VERSION_ INTEGER,
                    CREATE_TIME_ BIGINT,
                    TENANT_ID_ VARCHAR(255),
                    PARENT_DEPLOYMENT_ID_ VARCHAR(255),
                    PAYLOAD_ TEXT,
                    DATA_ TEXT NOT NULL
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#
            .to_string(),
        },
        // Event Registry Event Delivery table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_EVT_EVENT_DELIVERY".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_EVT_EVENT_DELIVERY (
                    ID_ TEXT PRIMARY KEY,
                    EVENT_DEF_ID_ TEXT,
                    EVENT_DEF_KEY_ TEXT,
                    EVENT_TYPE_ TEXT,
                    CHANNEL_KEY_ TEXT,
                    DIRECTION_ TEXT,
                    STATUS_ TEXT,
                    STATUS_HISTORY_ TEXT,
                    LAST_ERROR_ TEXT,
                    RETRY_COUNT_ INTEGER,
                    LAST_RETRY_AT_ INTEGER,
                    LAST_FAILURE_AT_ INTEGER,
                    NEXT_RETRY_AT_ INTEGER,
                    TENANT_ID_ TEXT,
                    PAYLOAD_ TEXT,
                    CREATED_AT_ INTEGER,
                    UPDATED_AT_ INTEGER,
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_EVT_EVENT_DELIVERY".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_EVT_EVENT_DELIVERY (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    EVENT_DEF_ID_ VARCHAR(255),
                    EVENT_DEF_KEY_ VARCHAR(255),
                    EVENT_TYPE_ VARCHAR(255),
                    CHANNEL_KEY_ VARCHAR(255),
                    DIRECTION_ VARCHAR(255),
                    STATUS_ VARCHAR(255),
                    STATUS_HISTORY_ TEXT,
                    LAST_ERROR_ TEXT,
                    RETRY_COUNT_ INTEGER,
                    LAST_RETRY_AT_ BIGINT,
                    LAST_FAILURE_AT_ BIGINT,
                    NEXT_RETRY_AT_ BIGINT,
                    TENANT_ID_ VARCHAR(255),
                    PAYLOAD_ TEXT,
                    CREATED_AT_ BIGINT,
                    UPDATED_AT_ BIGINT,
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_EVT_EVENT_DELIVERY".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_EVT_EVENT_DELIVERY (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    EVENT_DEF_ID_ VARCHAR(255),
                    EVENT_DEF_KEY_ VARCHAR(255),
                    EVENT_TYPE_ VARCHAR(255),
                    CHANNEL_KEY_ VARCHAR(255),
                    DIRECTION_ VARCHAR(255),
                    STATUS_ VARCHAR(255),
                    STATUS_HISTORY_ TEXT,
                    LAST_ERROR_ TEXT,
                    RETRY_COUNT_ INTEGER,
                    LAST_RETRY_AT_ BIGINT,
                    LAST_FAILURE_AT_ BIGINT,
                    NEXT_RETRY_AT_ BIGINT,
                    TENANT_ID_ VARCHAR(255),
                    PAYLOAD_ TEXT,
                    CREATED_AT_ BIGINT,
                    UPDATED_AT_ BIGINT,
                    DATA_ TEXT NOT NULL
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#
            .to_string(),
        },
        // Form Deployment table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_FORM_DEPLOYMENT".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_FORM_DEPLOYMENT (
                    ID_ TEXT PRIMARY KEY,
                    NAME_ TEXT,
                    DEPLOYED_AT_ INTEGER,
                    RESOURCE_NAMES_ TEXT,
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_FORM_DEPLOYMENT".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_FORM_DEPLOYMENT (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    NAME_ VARCHAR(255),
                    DEPLOYED_AT_ BIGINT,
                    RESOURCE_NAMES_ TEXT,
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_FORM_DEPLOYMENT".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_FORM_DEPLOYMENT (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    NAME_ VARCHAR(255),
                    DEPLOYED_AT_ BIGINT,
                    RESOURCE_NAMES_ TEXT,
                    DATA_ TEXT NOT NULL
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#
            .to_string(),
        },
        // Form Definition table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_FORM_DEFINITION".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_FORM_DEFINITION (
                    ID_ TEXT PRIMARY KEY,
                    DEPLOYMENT_ID_ TEXT,
                    KEY_ TEXT,
                    NAME_ TEXT,
                    VERSION_ INTEGER,
                    RESOURCE_NAME_ TEXT,
                    FORM_JSON_ TEXT,
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_FORM_DEFINITION".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_FORM_DEFINITION (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    DEPLOYMENT_ID_ VARCHAR(255),
                    KEY_ VARCHAR(255),
                    NAME_ VARCHAR(255),
                    VERSION_ INTEGER,
                    RESOURCE_NAME_ VARCHAR(255),
                    FORM_JSON_ TEXT,
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_FORM_DEFINITION".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_FORM_DEFINITION (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    DEPLOYMENT_ID_ VARCHAR(255),
                    KEY_ VARCHAR(255),
                    NAME_ VARCHAR(255),
                    VERSION_ INTEGER,
                    RESOURCE_NAME_ VARCHAR(255),
                    FORM_JSON_ TEXT,
                    DATA_ TEXT NOT NULL
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#
            .to_string(),
        },
        // Content Item table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CONTENT_ITEM".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CONTENT_ITEM (
                    ID_ TEXT PRIMARY KEY,
                    NAME_ TEXT,
                    MIME_TYPE_ TEXT,
                    CREATED_AT_ INTEGER,
                    CONTENT_ BLOB,
                    METADATA_ TEXT,
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CONTENT_ITEM".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CONTENT_ITEM (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    NAME_ VARCHAR(255),
                    MIME_TYPE_ VARCHAR(255),
                    CREATED_AT_ BIGINT,
                    CONTENT_ BYTEA,
                    METADATA_ TEXT,
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CONTENT_ITEM".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CONTENT_ITEM (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    NAME_ VARCHAR(255),
                    MIME_TYPE_ VARCHAR(255),
                    CREATED_AT_ BIGINT,
                    CONTENT_ LONGBLOB,
                    METADATA_ TEXT,
                    DATA_ TEXT NOT NULL
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#
            .to_string(),
        },
        // HTTP Task Record table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_HTTP_TASK_RECORD".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_HTTP_TASK_RECORD (
                    ID_ TEXT PRIMARY KEY,
                    PROC_INST_ID_ TEXT,
                    EXECUTION_ID_ TEXT,
                    ACTIVITY_ID_ TEXT,
                    METHOD_ TEXT,
                    URL_ TEXT,
                    REQUEST_BODY_ TEXT,
                    RESPONSE_STATUS_CODE_ INTEGER,
                    RESPONSE_BODY_ TEXT,
                    STATUS_ TEXT,
                    CREATED_AT_ INTEGER,
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_HTTP_TASK_RECORD".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_HTTP_TASK_RECORD (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    PROC_INST_ID_ VARCHAR(255),
                    EXECUTION_ID_ VARCHAR(255),
                    ACTIVITY_ID_ VARCHAR(255),
                    METHOD_ VARCHAR(255),
                    URL_ VARCHAR(2000),
                    REQUEST_BODY_ TEXT,
                    RESPONSE_STATUS_CODE_ INTEGER,
                    RESPONSE_BODY_ TEXT,
                    STATUS_ VARCHAR(255),
                    CREATED_AT_ BIGINT,
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_HTTP_TASK_RECORD".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_HTTP_TASK_RECORD (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    PROC_INST_ID_ VARCHAR(255),
                    EXECUTION_ID_ VARCHAR(255),
                    ACTIVITY_ID_ VARCHAR(255),
                    METHOD_ VARCHAR(255),
                    URL_ VARCHAR(2000),
                    REQUEST_BODY_ TEXT,
                    RESPONSE_STATUS_CODE_ INTEGER,
                    RESPONSE_BODY_ TEXT,
                    STATUS_ VARCHAR(255),
                    CREATED_AT_ BIGINT,
                    DATA_ TEXT NOT NULL
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#
            .to_string(),
        },
        // Mail Outbox table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_MAIL_OUTBOX".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_MAIL_OUTBOX (
                    ID_ TEXT PRIMARY KEY,
                    PROC_INST_ID_ TEXT,
                    EXECUTION_ID_ TEXT,
                    ACTIVITY_ID_ TEXT,
                    RECIPIENT_ TEXT,
                    SUBJECT_ TEXT,
                    BODY_ TEXT,
                    STATUS_ TEXT,
                    CREATED_AT_ INTEGER,
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_MAIL_OUTBOX".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_MAIL_OUTBOX (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    PROC_INST_ID_ VARCHAR(255),
                    EXECUTION_ID_ VARCHAR(255),
                    ACTIVITY_ID_ VARCHAR(255),
                    RECIPIENT_ TEXT,
                    SUBJECT_ VARCHAR(255),
                    BODY_ TEXT,
                    STATUS_ VARCHAR(255),
                    CREATED_AT_ BIGINT,
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_MAIL_OUTBOX".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_MAIL_OUTBOX (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    PROC_INST_ID_ VARCHAR(255),
                    EXECUTION_ID_ VARCHAR(255),
                    ACTIVITY_ID_ VARCHAR(255),
                    RECIPIENT_ TEXT,
                    SUBJECT_ VARCHAR(255),
                    BODY_ TEXT,
                    STATUS_ VARCHAR(255),
                    CREATED_AT_ BIGINT,
                    DATA_ TEXT NOT NULL
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#
            .to_string(),
        },
        // Event Wait State table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RU_EVENT_WAIT_STATE".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RU_EVENT_WAIT_STATE (
                    ID_ TEXT PRIMARY KEY,
                    PROC_INST_ID_ TEXT,
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RU_EVENT_WAIT_STATE".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RU_EVENT_WAIT_STATE (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    PROC_INST_ID_ VARCHAR(255),
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RU_EVENT_WAIT_STATE".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RU_EVENT_WAIT_STATE (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    PROC_INST_ID_ VARCHAR(255),
                    DATA_ TEXT NOT NULL
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#
            .to_string(),
        },
        // Boundary Event State table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RU_BOUNDARY_EVENT_STATE".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RU_BOUNDARY_EVENT_STATE (
                    ID_ TEXT PRIMARY KEY,
                    PROC_INST_ID_ TEXT,
                    HOST_EXECUTION_ID_ TEXT,
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RU_BOUNDARY_EVENT_STATE".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RU_BOUNDARY_EVENT_STATE (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    PROC_INST_ID_ VARCHAR(255),
                    HOST_EXECUTION_ID_ VARCHAR(255),
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RU_BOUNDARY_EVENT_STATE".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RU_BOUNDARY_EVENT_STATE (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    PROC_INST_ID_ VARCHAR(255),
                    HOST_EXECUTION_ID_ VARCHAR(255),
                    DATA_ TEXT NOT NULL
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#
            .to_string(),
        },
        // Event Subprocess Timer Subscription table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RU_EVT_SUBPROC_TIMER".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RU_EVT_SUBPROC_TIMER (
                    ID_ TEXT PRIMARY KEY,
                    PROC_INST_ID_ TEXT,
                    EXECUTION_ID_ TEXT,
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RU_EVT_SUBPROC_TIMER".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RU_EVT_SUBPROC_TIMER (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    PROC_INST_ID_ VARCHAR(255),
                    EXECUTION_ID_ VARCHAR(255),
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RU_EVT_SUBPROC_TIMER".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RU_EVT_SUBPROC_TIMER (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    PROC_INST_ID_ VARCHAR(255),
                    EXECUTION_ID_ VARCHAR(255),
                    DATA_ TEXT NOT NULL
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#
            .to_string(),
        },
        // Event Subprocess Event Subscription table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RU_EVT_SUBPROC_EVENT".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RU_EVT_SUBPROC_EVENT (
                    ID_ TEXT PRIMARY KEY,
                    PROC_INST_ID_ TEXT,
                    EXECUTION_ID_ TEXT,
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RU_EVT_SUBPROC_EVENT".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RU_EVT_SUBPROC_EVENT (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    PROC_INST_ID_ VARCHAR(255),
                    EXECUTION_ID_ VARCHAR(255),
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RU_EVT_SUBPROC_EVENT".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RU_EVT_SUBPROC_EVENT (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    PROC_INST_ID_ VARCHAR(255),
                    EXECUTION_ID_ VARCHAR(255),
                    DATA_ TEXT NOT NULL
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#
            .to_string(),
        },
        // Signal Subscription table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RU_SIGNAL_SUBSCR".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RU_SIGNAL_SUBSCR (
                    ID_ TEXT PRIMARY KEY,
                    EXECUTION_ID_ TEXT,
                    PROC_INST_ID_ TEXT,
                    SIGNAL_NAME_ TEXT,
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RU_SIGNAL_SUBSCR".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RU_SIGNAL_SUBSCR (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    EXECUTION_ID_ VARCHAR(255),
                    PROC_INST_ID_ VARCHAR(255),
                    SIGNAL_NAME_ VARCHAR(255),
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RU_SIGNAL_SUBSCR".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RU_SIGNAL_SUBSCR (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    EXECUTION_ID_ VARCHAR(255),
                    PROC_INST_ID_ VARCHAR(255),
                    SIGNAL_NAME_ VARCHAR(255),
                    DATA_ TEXT NOT NULL
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#
            .to_string(),
        },
        // External Worker Job table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RU_EXT_WORKER_JOB".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RU_EXT_WORKER_JOB (
                    ID_ TEXT PRIMARY KEY,
                    PROC_INST_ID_ TEXT,
                    EXECUTION_ID_ TEXT,
                    TASK_ID_ TEXT,
                    HANDLER_TYPE_ TEXT,
                    LOCK_OWNER_ TEXT,
                    LOCK_TIME_ INTEGER,
                    LOCK_EXP_TIME_ INTEGER,
                    RETRIES_ INTEGER,
                    ERROR_MSG_ TEXT,
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RU_EXT_WORKER_JOB".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RU_EXT_WORKER_JOB (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    PROC_INST_ID_ VARCHAR(255),
                    EXECUTION_ID_ VARCHAR(255),
                    TASK_ID_ VARCHAR(255),
                    HANDLER_TYPE_ VARCHAR(255),
                    LOCK_OWNER_ VARCHAR(255),
                    LOCK_TIME_ BIGINT,
                    LOCK_EXP_TIME_ BIGINT,
                    RETRIES_ INTEGER,
                    ERROR_MSG_ TEXT,
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RU_EXT_WORKER_JOB".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RU_EXT_WORKER_JOB (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    PROC_INST_ID_ VARCHAR(255),
                    EXECUTION_ID_ VARCHAR(255),
                    TASK_ID_ VARCHAR(255),
                    HANDLER_TYPE_ VARCHAR(255),
                    LOCK_OWNER_ VARCHAR(255),
                    LOCK_TIME_ BIGINT,
                    LOCK_EXP_TIME_ BIGINT,
                    RETRIES_ INTEGER,
                    ERROR_MSG_ TEXT,
                    DATA_ TEXT NOT NULL
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#
            .to_string(),
        },
        // Batch Part table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RU_BATCH_PART".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RU_BATCH_PART (
                    ID_ TEXT PRIMARY KEY,
                    BATCH_ID_ TEXT,
                    TYPE_ TEXT,
                    STATUS_ TEXT,
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RU_BATCH_PART".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RU_BATCH_PART (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    BATCH_ID_ VARCHAR(255),
                    TYPE_ VARCHAR(255),
                    STATUS_ VARCHAR(255),
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RU_BATCH_PART".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RU_BATCH_PART (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    BATCH_ID_ VARCHAR(255),
                    TYPE_ VARCHAR(255),
                    STATUS_ VARCHAR(255),
                    DATA_ TEXT NOT NULL
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#
            .to_string(),
        },
        // Process Definition Version table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RE_PROCDEF_VERSION".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RE_PROCDEF_VERSION (
                    ID_ TEXT PRIMARY KEY,
                    TENANT_ID_ TEXT,
                    PROC_KEY_ TEXT,
                    VERSION_ INTEGER
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RE_PROCDEF_VERSION".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RE_PROCDEF_VERSION (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    TENANT_ID_ VARCHAR(255),
                    PROC_KEY_ VARCHAR(255),
                    VERSION_ INTEGER
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RE_PROCDEF_VERSION".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RE_PROCDEF_VERSION (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    TENANT_ID_ VARCHAR(255),
                    PROC_KEY_ VARCHAR(255),
                    VERSION_ INTEGER
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#
            .to_string(),
        },
        // Deployment Resource table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_DE_RES".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_DE_RES (
                    ID_ TEXT PRIMARY KEY,
                    DEPLOYMENT_ID_ TEXT,
                    NAME_ TEXT,
                    RESOURCE_TYPE_ TEXT,
                    CONTENT_TYPE_ TEXT,
                    CREATED_AT_ INTEGER,
                    BYTES_ BLOB,
                    DATA_ TEXT
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_DE_RES".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_DE_RES (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    DEPLOYMENT_ID_ VARCHAR(255),
                    NAME_ VARCHAR(255),
                    RESOURCE_TYPE_ VARCHAR(255),
                    CONTENT_TYPE_ VARCHAR(255),
                    CREATED_AT_ BIGINT,
                    BYTES_ BYTEA,
                    DATA_ TEXT
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_DE_RES".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_DE_RES (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    DEPLOYMENT_ID_ VARCHAR(255),
                    NAME_ VARCHAR(255),
                    RESOURCE_TYPE_ VARCHAR(255),
                    CONTENT_TYPE_ VARCHAR(255),
                    CREATED_AT_ BIGINT,
                    BYTES_ LONGBLOB,
                    DATA_ TEXT
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#
            .to_string(),
        },
        // Repository Model table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RE_REPOSITORY_MODEL".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RE_REPOSITORY_MODEL (
                    ID_ TEXT PRIMARY KEY,
                    DEPLOYMENT_ID_ TEXT,
                    MODEL_KEY_ TEXT,
                    TENANT_ID_ TEXT,
                    SOURCE_BYTES_ BLOB NOT NULL,
                    SOURCE_EXTRA_BYTES_ BLOB NOT NULL,
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RE_REPOSITORY_MODEL".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RE_REPOSITORY_MODEL (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    DEPLOYMENT_ID_ VARCHAR(255),
                    MODEL_KEY_ VARCHAR(255),
                    TENANT_ID_ VARCHAR(255),
                    SOURCE_BYTES_ BYTEA NOT NULL,
                    SOURCE_EXTRA_BYTES_ BYTEA NOT NULL,
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_RE_REPOSITORY_MODEL".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_RE_REPOSITORY_MODEL (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    DEPLOYMENT_ID_ VARCHAR(255),
                    MODEL_KEY_ VARCHAR(255),
                    TENANT_ID_ VARCHAR(255),
                    SOURCE_BYTES_ LONGBLOB NOT NULL,
                    SOURCE_EXTRA_BYTES_ LONGBLOB NOT NULL,
                    DATA_ TEXT NOT NULL
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#
            .to_string(),
        },
        // Timer Coordinator State table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_COORD_TIMER_STATE".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_COORD_TIMER_STATE (
                    ID_ TEXT PRIMARY KEY,
                    OWNER_ID_ TEXT,
                    FENCING_TOKEN_ INTEGER,
                    LAST_HEARTBEAT_ INTEGER,
                    STATE_ TEXT
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_COORD_TIMER_STATE".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_COORD_TIMER_STATE (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    OWNER_ID_ VARCHAR(255),
                    FENCING_TOKEN_ BIGINT,
                    LAST_HEARTBEAT_ BIGINT,
                    STATE_ TEXT
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_COORD_TIMER_STATE".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_COORD_TIMER_STATE (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    OWNER_ID_ VARCHAR(255),
                    FENCING_TOKEN_ BIGINT,
                    LAST_HEARTBEAT_ BIGINT,
                    STATE_ TEXT
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#
            .to_string(),
        },
        // Timer Node table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_COORD_TIMER_NODE".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_COORD_TIMER_NODE (
                    ID_ TEXT PRIMARY KEY,
                    OWNER_ID_ TEXT,
                    LAST_HEARTBEAT_ INTEGER,
                    STATE_ TEXT
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_COORD_TIMER_NODE".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_COORD_TIMER_NODE (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    OWNER_ID_ VARCHAR(255),
                    LAST_HEARTBEAT_ BIGINT,
                    STATE_ TEXT
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_COORD_TIMER_NODE".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_COORD_TIMER_NODE (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    OWNER_ID_ VARCHAR(255),
                    LAST_HEARTBEAT_ BIGINT,
                    STATE_ TEXT
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#
            .to_string(),
        },
    ]
}

pub fn get_dmn_scripts() -> Vec<SchemaScript> {
    vec![
        // DMN Deployment table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_DMN_DEPLOYMENT".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_DMN_DEPLOYMENT (
                    ID_ TEXT PRIMARY KEY,
                    NAME_ TEXT NOT NULL,
                    CATEGORY_ TEXT,
                    PARENT_DEPLOYMENT_ID_ TEXT,
                    TENANT_ID_ TEXT,
                    DEPLOYED_AT_ TEXT NOT NULL,
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_DMN_DEPLOYMENT".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_DMN_DEPLOYMENT (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    NAME_ VARCHAR(255) NOT NULL,
                    CATEGORY_ VARCHAR(255),
                    PARENT_DEPLOYMENT_ID_ VARCHAR(255),
                    TENANT_ID_ VARCHAR(255),
                    DEPLOYED_AT_ VARCHAR(255) NOT NULL,
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_DMN_DEPLOYMENT".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_DMN_DEPLOYMENT (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    NAME_ VARCHAR(255) NOT NULL,
                    CATEGORY_ VARCHAR(255),
                    PARENT_DEPLOYMENT_ID_ VARCHAR(255),
                    TENANT_ID_ VARCHAR(255),
                    DEPLOYED_AT_ VARCHAR(255) NOT NULL,
                    DATA_ TEXT NOT NULL
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#
            .to_string(),
        },
        // DMN Decision table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_DMN_DECISION".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_DMN_DECISION (
                    ID_ TEXT PRIMARY KEY,
                    DECISION_KEY_ TEXT NOT NULL,
                    DEPLOYMENT_ID_ TEXT NOT NULL,
                    TENANT_ID_ TEXT,
                    VERSION_ INTEGER NOT NULL,
                    RESOURCE_NAME_ TEXT NOT NULL,
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_DMN_DECISION".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_DMN_DECISION (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    DECISION_KEY_ VARCHAR(255) NOT NULL,
                    DEPLOYMENT_ID_ VARCHAR(255) NOT NULL,
                    TENANT_ID_ VARCHAR(255),
                    VERSION_ INTEGER NOT NULL,
                    RESOURCE_NAME_ VARCHAR(255) NOT NULL,
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_DMN_DECISION".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_DMN_DECISION (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    DECISION_KEY_ VARCHAR(255) NOT NULL,
                    DEPLOYMENT_ID_ VARCHAR(255) NOT NULL,
                    TENANT_ID_ VARCHAR(255),
                    VERSION_ INTEGER NOT NULL,
                    RESOURCE_NAME_ VARCHAR(255) NOT NULL,
                    DATA_ TEXT NOT NULL
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#
            .to_string(),
        },
        // DMN Resource table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_DMN_RESOURCE".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_DMN_RESOURCE (
                    DEPLOYMENT_ID_ TEXT NOT NULL,
                    RESOURCE_NAME_ TEXT NOT NULL,
                    RESOURCE_TYPE_ TEXT NOT NULL DEFAULT 'resource',
                    CONTENT_TYPE_ TEXT NOT NULL DEFAULT 'application/octet-stream',
                    BYTES_ BLOB NOT NULL,
                    CREATED_AT_ INTEGER NOT NULL DEFAULT 0,
                    PRIMARY KEY (DEPLOYMENT_ID_, RESOURCE_NAME_)
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_DMN_RESOURCE".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_DMN_RESOURCE (
                    DEPLOYMENT_ID_ VARCHAR(255) NOT NULL,
                    RESOURCE_NAME_ VARCHAR(255) NOT NULL,
                    RESOURCE_TYPE_ VARCHAR(255) NOT NULL DEFAULT 'resource',
                    CONTENT_TYPE_ VARCHAR(255) NOT NULL DEFAULT 'application/octet-stream',
                    BYTES_ BYTEA NOT NULL,
                    CREATED_AT_ BIGINT NOT NULL DEFAULT 0,
                    PRIMARY KEY (DEPLOYMENT_ID_, RESOURCE_NAME_)
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_DMN_RESOURCE".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_DMN_RESOURCE (
                    DEPLOYMENT_ID_ VARCHAR(255) NOT NULL,
                    RESOURCE_NAME_ VARCHAR(255) NOT NULL,
                    RESOURCE_TYPE_ VARCHAR(255) NOT NULL DEFAULT 'resource',
                    CONTENT_TYPE_ VARCHAR(255) NOT NULL DEFAULT 'application/octet-stream',
                    BYTES_ LONGBLOB NOT NULL,
                    CREATED_AT_ BIGINT NOT NULL DEFAULT 0,
                    PRIMARY KEY (DEPLOYMENT_ID_, RESOURCE_NAME_)
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#
            .to_string(),
        },
        // DMN DRD table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_DMN_DRD".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_DMN_DRD (
                    ID_ TEXT PRIMARY KEY,
                    NAME_ TEXT NOT NULL,
                    DEPLOYMENT_ID_ TEXT NOT NULL,
                    RESOURCE_NAME_ TEXT NOT NULL,
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_DMN_DRD".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_DMN_DRD (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    NAME_ VARCHAR(255) NOT NULL,
                    DEPLOYMENT_ID_ VARCHAR(255) NOT NULL,
                    RESOURCE_NAME_ VARCHAR(255) NOT NULL,
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_DMN_DRD".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_DMN_DRD (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    NAME_ VARCHAR(255) NOT NULL,
                    DEPLOYMENT_ID_ VARCHAR(255) NOT NULL,
                    RESOURCE_NAME_ VARCHAR(255) NOT NULL,
                    DATA_ TEXT NOT NULL
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#
            .to_string(),
        },
        // DMN Execution History table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_DMN_HI_EXECUTION".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_DMN_HI_EXECUTION (
                    EXECUTION_ID_ TEXT PRIMARY KEY,
                    DECISION_KEY_ TEXT NOT NULL,
                    DECISION_DEFINITION_ID_ TEXT NOT NULL,
                    DEPLOYMENT_ID_ TEXT NOT NULL,
                    BUSINESS_KEY_ TEXT,
                    TENANT_ID_ TEXT,
                    INSTANCE_ID_ TEXT,
                    SCOPE_EXECUTION_ID_ TEXT,
                    ACTIVITY_ID_ TEXT,
                    SCOPE_TYPE_ TEXT,
                    EXECUTED_AT_ TEXT NOT NULL,
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_DMN_HI_EXECUTION".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_DMN_HI_EXECUTION (
                    EXECUTION_ID_ VARCHAR(255) PRIMARY KEY,
                    DECISION_KEY_ VARCHAR(255) NOT NULL,
                    DECISION_DEFINITION_ID_ VARCHAR(255) NOT NULL,
                    DEPLOYMENT_ID_ VARCHAR(255) NOT NULL,
                    BUSINESS_KEY_ VARCHAR(255),
                    TENANT_ID_ VARCHAR(255),
                    INSTANCE_ID_ VARCHAR(255),
                    SCOPE_EXECUTION_ID_ VARCHAR(255),
                    ACTIVITY_ID_ VARCHAR(255),
                    SCOPE_TYPE_ VARCHAR(255),
                    EXECUTED_AT_ VARCHAR(255) NOT NULL,
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_DMN_HI_EXECUTION".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_DMN_HI_EXECUTION (
                    EXECUTION_ID_ VARCHAR(255) PRIMARY KEY,
                    DECISION_KEY_ VARCHAR(255) NOT NULL,
                    DECISION_DEFINITION_ID_ VARCHAR(255) NOT NULL,
                    DEPLOYMENT_ID_ VARCHAR(255) NOT NULL,
                    BUSINESS_KEY_ VARCHAR(255),
                    TENANT_ID_ VARCHAR(255),
                    INSTANCE_ID_ VARCHAR(255),
                    SCOPE_EXECUTION_ID_ VARCHAR(255),
                    ACTIVITY_ID_ VARCHAR(255),
                    SCOPE_TYPE_ VARCHAR(255),
                    EXECUTED_AT_ VARCHAR(255) NOT NULL,
                    DATA_ TEXT NOT NULL
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#
            .to_string(),
        },
    ]
}

pub fn get_app_scripts() -> Vec<SchemaScript> {
    vec![
        // App Deployment table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_APP_DEPLOYMENT".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_APP_DEPLOYMENT (
                    ID_ TEXT PRIMARY KEY,
                    NAME_ TEXT NOT NULL,
                    CATEGORY_ TEXT,
                    TENANT_ID_ TEXT,
                    DEPLOYED_AT_ TEXT NOT NULL,
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_APP_DEPLOYMENT".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_APP_DEPLOYMENT (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    NAME_ VARCHAR(255) NOT NULL,
                    CATEGORY_ VARCHAR(255),
                    TENANT_ID_ VARCHAR(255),
                    DEPLOYED_AT_ VARCHAR(255) NOT NULL,
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_APP_DEPLOYMENT".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_APP_DEPLOYMENT (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    NAME_ VARCHAR(255) NOT NULL,
                    CATEGORY_ VARCHAR(255),
                    TENANT_ID_ VARCHAR(255),
                    DEPLOYED_AT_ VARCHAR(255) NOT NULL,
                    DATA_ TEXT NOT NULL
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#
            .to_string(),
        },
        // App Definition table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_APP_DEFINITION".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_APP_DEFINITION (
                    ID_ TEXT PRIMARY KEY,
                    APP_KEY_ TEXT NOT NULL,
                    DEPLOYMENT_ID_ TEXT NOT NULL,
                    TENANT_ID_ TEXT,
                    VERSION_ INTEGER NOT NULL,
                    RESOURCE_NAME_ TEXT NOT NULL,
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_APP_DEFINITION".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_APP_DEFINITION (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    APP_KEY_ VARCHAR(255) NOT NULL,
                    DEPLOYMENT_ID_ VARCHAR(255) NOT NULL,
                    TENANT_ID_ VARCHAR(255),
                    VERSION_ INTEGER NOT NULL,
                    RESOURCE_NAME_ VARCHAR(255) NOT NULL,
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_APP_DEFINITION".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_APP_DEFINITION (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    APP_KEY_ VARCHAR(255) NOT NULL,
                    DEPLOYMENT_ID_ VARCHAR(255) NOT NULL,
                    TENANT_ID_ VARCHAR(255),
                    VERSION_ INTEGER NOT NULL,
                    RESOURCE_NAME_ VARCHAR(255) NOT NULL,
                    DATA_ TEXT NOT NULL
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#
            .to_string(),
        },
        // App Resolved Composition table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_APP_RESOLVED_COMPOSITION".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_APP_RESOLVED_COMPOSITION (
                    ID_ TEXT PRIMARY KEY,
                    APP_DEFINITION_ID_ TEXT NOT NULL,
                    APP_KEY_ TEXT NOT NULL,
                    DEPLOYMENT_ID_ TEXT NOT NULL,
                    TENANT_ID_ TEXT,
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_APP_RESOLVED_COMPOSITION".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_APP_RESOLVED_COMPOSITION (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    APP_DEFINITION_ID_ VARCHAR(255) NOT NULL,
                    APP_KEY_ VARCHAR(255) NOT NULL,
                    DEPLOYMENT_ID_ VARCHAR(255) NOT NULL,
                    TENANT_ID_ VARCHAR(255),
                    DATA_ TEXT NOT NULL
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_APP_RESOLVED_COMPOSITION".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_APP_RESOLVED_COMPOSITION (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    APP_DEFINITION_ID_ VARCHAR(255) NOT NULL,
                    APP_KEY_ VARCHAR(255) NOT NULL,
                    DEPLOYMENT_ID_ VARCHAR(255) NOT NULL,
                    TENANT_ID_ VARCHAR(255),
                    DATA_ TEXT NOT NULL
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#
            .to_string(),
        },
        // App Deployment Resource table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_APP_DEPLOYMENT_RESOURCE".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_APP_DEPLOYMENT_RESOURCE (
                    DEPLOYMENT_ID_ TEXT NOT NULL,
                    RESOURCE_NAME_ TEXT NOT NULL,
                    RESOURCE_TYPE_ TEXT NOT NULL,
                    CONTENT_TYPE_ TEXT NOT NULL,
                    BYTES_ BLOB NOT NULL,
                    CREATED_AT_ INTEGER NOT NULL,
                    PRIMARY KEY (DEPLOYMENT_ID_, RESOURCE_NAME_)
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_APP_DEPLOYMENT_RESOURCE".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_APP_DEPLOYMENT_RESOURCE (
                    DEPLOYMENT_ID_ VARCHAR(255) NOT NULL,
                    RESOURCE_NAME_ VARCHAR(255) NOT NULL,
                    RESOURCE_TYPE_ VARCHAR(255) NOT NULL,
                    CONTENT_TYPE_ VARCHAR(255) NOT NULL,
                    BYTES_ BYTEA NOT NULL,
                    CREATED_AT_ BIGINT NOT NULL,
                    PRIMARY KEY (DEPLOYMENT_ID_, RESOURCE_NAME_)
                )
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_APP_DEPLOYMENT_RESOURCE".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_APP_DEPLOYMENT_RESOURCE (
                    DEPLOYMENT_ID_ VARCHAR(255) NOT NULL,
                    RESOURCE_NAME_ VARCHAR(255) NOT NULL,
                    RESOURCE_TYPE_ VARCHAR(255) NOT NULL,
                    CONTENT_TYPE_ VARCHAR(255) NOT NULL,
                    BYTES_ LONGBLOB NOT NULL,
                    CREATED_AT_ BIGINT NOT NULL,
                    PRIMARY KEY (DEPLOYMENT_ID_, RESOURCE_NAME_)
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#
            .to_string(),
        },
    ]
}

pub fn get_cmmn_scripts() -> Vec<SchemaScript> {
    vec![
        // CMMN Deployment table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_DEPLOYMENT".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_DEPLOYMENT (
                    ID_ TEXT PRIMARY KEY,
                    NAME_ TEXT NOT NULL,
                    TENANT_ID_ TEXT,
                    DEPLOYED_AT_ TEXT NOT NULL,
                    DATA_ TEXT NOT NULL
                )
            "#.to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_DEPLOYMENT".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_DEPLOYMENT (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    NAME_ VARCHAR(255) NOT NULL,
                    TENANT_ID_ VARCHAR(255),
                    DEPLOYED_AT_ VARCHAR(255) NOT NULL,
                    DATA_ TEXT NOT NULL
                )
            "#.to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_DEPLOYMENT".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_DEPLOYMENT (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    NAME_ VARCHAR(255) NOT NULL,
                    TENANT_ID_ VARCHAR(255),
                    DEPLOYED_AT_ VARCHAR(255) NOT NULL,
                    DATA_ TEXT NOT NULL
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#.to_string(),
        },
        // CMMN Case Definition table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_CASE_DEFINITION".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_CASE_DEFINITION (
                    ID_ TEXT PRIMARY KEY,
                    CASE_KEY_ TEXT NOT NULL,
                    DEPLOYMENT_ID_ TEXT NOT NULL,
                    TENANT_ID_ TEXT,
                    VERSION_ INTEGER NOT NULL,
                    RESOURCE_NAME_ TEXT NOT NULL,
                    DATA_ TEXT NOT NULL
                )
            "#.to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_CASE_DEFINITION".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_CASE_DEFINITION (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    CASE_KEY_ VARCHAR(255) NOT NULL,
                    DEPLOYMENT_ID_ VARCHAR(255) NOT NULL,
                    TENANT_ID_ VARCHAR(255),
                    VERSION_ INTEGER NOT NULL,
                    RESOURCE_NAME_ VARCHAR(255) NOT NULL,
                    DATA_ TEXT NOT NULL
                )
            "#.to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_CASE_DEFINITION".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_CASE_DEFINITION (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    CASE_KEY_ VARCHAR(255) NOT NULL,
                    DEPLOYMENT_ID_ VARCHAR(255) NOT NULL,
                    TENANT_ID_ VARCHAR(255),
                    VERSION_ INTEGER NOT NULL,
                    RESOURCE_NAME_ VARCHAR(255) NOT NULL,
                    DATA_ TEXT NOT NULL
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#.to_string(),
        },
        // CMMN Deployment Resource table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_DEPLOYMENT_RESOURCE".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_DEPLOYMENT_RESOURCE (
                    DEPLOYMENT_ID_ TEXT NOT NULL,
                    RESOURCE_NAME_ TEXT NOT NULL,
                    RESOURCE_TYPE_ TEXT NOT NULL DEFAULT 'resource',
                    CONTENT_TYPE_ TEXT NOT NULL DEFAULT 'application/octet-stream',
                    BYTES_ BLOB NOT NULL,
                    CREATED_AT_ INTEGER NOT NULL DEFAULT 0,
                    PRIMARY KEY (DEPLOYMENT_ID_, RESOURCE_NAME_)
                )
            "#.to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_DEPLOYMENT_RESOURCE".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_DEPLOYMENT_RESOURCE (
                    DEPLOYMENT_ID_ VARCHAR(255) NOT NULL,
                    RESOURCE_NAME_ VARCHAR(255) NOT NULL,
                    RESOURCE_TYPE_ VARCHAR(255) NOT NULL DEFAULT 'resource',
                    CONTENT_TYPE_ VARCHAR(255) NOT NULL DEFAULT 'application/octet-stream',
                    BYTES_ BYTEA NOT NULL,
                    CREATED_AT_ BIGINT NOT NULL DEFAULT 0,
                    PRIMARY KEY (DEPLOYMENT_ID_, RESOURCE_NAME_)
                )
            "#.to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_DEPLOYMENT_RESOURCE".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_DEPLOYMENT_RESOURCE (
                    DEPLOYMENT_ID_ VARCHAR(255) NOT NULL,
                    RESOURCE_NAME_ VARCHAR(255) NOT NULL,
                    RESOURCE_TYPE_ VARCHAR(255) NOT NULL DEFAULT 'resource',
                    CONTENT_TYPE_ VARCHAR(255) NOT NULL DEFAULT 'application/octet-stream',
                    BYTES_ LONGBLOB NOT NULL,
                    CREATED_AT_ BIGINT NOT NULL DEFAULT 0,
                    PRIMARY KEY (DEPLOYMENT_ID_, RESOURCE_NAME_)
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#.to_string(),
        },
        // CMMN Case Instance table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_CASE_INSTANCE".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_CASE_INSTANCE (
                    ID_ TEXT PRIMARY KEY,
                    CASE_DEFINITION_ID_ TEXT NOT NULL,
                    CASE_KEY_ TEXT NOT NULL,
                    TENANT_ID_ TEXT,
                    BUSINESS_KEY_ TEXT,
                    STATE_ TEXT NOT NULL,
                    STARTED_AT_ TEXT NOT NULL,
                    DATA_ TEXT NOT NULL
                )
            "#.to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_CASE_INSTANCE".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_CASE_INSTANCE (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    CASE_DEFINITION_ID_ VARCHAR(255) NOT NULL,
                    CASE_KEY_ VARCHAR(255) NOT NULL,
                    TENANT_ID_ VARCHAR(255),
                    BUSINESS_KEY_ VARCHAR(255),
                    STATE_ VARCHAR(255) NOT NULL,
                    STARTED_AT_ VARCHAR(255) NOT NULL,
                    DATA_ TEXT NOT NULL
                )
            "#.to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_CASE_INSTANCE".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_CASE_INSTANCE (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    CASE_DEFINITION_ID_ VARCHAR(255) NOT NULL,
                    CASE_KEY_ VARCHAR(255) NOT NULL,
                    TENANT_ID_ VARCHAR(255),
                    BUSINESS_KEY_ VARCHAR(255),
                    STATE_ VARCHAR(255) NOT NULL,
                    STARTED_AT_ VARCHAR(255) NOT NULL,
                    DATA_ TEXT NOT NULL
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#.to_string(),
        },
        // CMMN Stage Instance table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_STAGE_INSTANCE".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_STAGE_INSTANCE (
                    ID_ TEXT PRIMARY KEY,
                    CASE_INSTANCE_ID_ TEXT NOT NULL,
                    PARENT_STAGE_INSTANCE_ID_ TEXT,
                    STAGE_DEFINITION_ID_ TEXT NOT NULL,
                    STATE_ TEXT NOT NULL,
                    ACTIVATED_AT_ TEXT NOT NULL,
                    DATA_ TEXT NOT NULL
                )
            "#.to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_STAGE_INSTANCE".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_STAGE_INSTANCE (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    CASE_INSTANCE_ID_ VARCHAR(255) NOT NULL,
                    PARENT_STAGE_INSTANCE_ID_ VARCHAR(255),
                    STAGE_DEFINITION_ID_ VARCHAR(255) NOT NULL,
                    STATE_ VARCHAR(255) NOT NULL,
                    ACTIVATED_AT_ VARCHAR(255) NOT NULL,
                    DATA_ TEXT NOT NULL
                )
            "#.to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_STAGE_INSTANCE".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_STAGE_INSTANCE (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    CASE_INSTANCE_ID_ VARCHAR(255) NOT NULL,
                    PARENT_STAGE_INSTANCE_ID_ VARCHAR(255),
                    STAGE_DEFINITION_ID_ VARCHAR(255) NOT NULL,
                    STATE_ VARCHAR(255) NOT NULL,
                    ACTIVATED_AT_ VARCHAR(255) NOT NULL,
                    DATA_ TEXT NOT NULL
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#.to_string(),
        },
        // CMMN Stage History table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_STAGE_HISTORY".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_STAGE_HISTORY (
                    ID_ TEXT PRIMARY KEY,
                    CASE_INSTANCE_ID_ TEXT NOT NULL,
                    CASE_DEFINITION_ID_ TEXT NOT NULL,
                    PARENT_STAGE_INSTANCE_ID_ TEXT,
                    STAGE_DEFINITION_ID_ TEXT NOT NULL,
                    STATE_ TEXT NOT NULL,
                    ACTIVATED_AT_ TEXT NOT NULL,
                    ENDED_AT_ TEXT,
                    DATA_ TEXT NOT NULL
                )
            "#.to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_STAGE_HISTORY".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_STAGE_HISTORY (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    CASE_INSTANCE_ID_ VARCHAR(255) NOT NULL,
                    CASE_DEFINITION_ID_ VARCHAR(255) NOT NULL,
                    PARENT_STAGE_INSTANCE_ID_ VARCHAR(255),
                    STAGE_DEFINITION_ID_ VARCHAR(255) NOT NULL,
                    STATE_ VARCHAR(255) NOT NULL,
                    ACTIVATED_AT_ VARCHAR(255) NOT NULL,
                    ENDED_AT_ VARCHAR(255),
                    DATA_ TEXT NOT NULL
                )
            "#.to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_STAGE_HISTORY".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_STAGE_HISTORY (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    CASE_INSTANCE_ID_ VARCHAR(255) NOT NULL,
                    CASE_DEFINITION_ID_ VARCHAR(255) NOT NULL,
                    PARENT_STAGE_INSTANCE_ID_ VARCHAR(255),
                    STAGE_DEFINITION_ID_ VARCHAR(255) NOT NULL,
                    STATE_ VARCHAR(255) NOT NULL,
                    ACTIVATED_AT_ VARCHAR(255) NOT NULL,
                    ENDED_AT_ VARCHAR(255),
                    DATA_ TEXT NOT NULL
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#.to_string(),
        },
        // CMMN Human Task table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_HUMAN_TASK".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_HUMAN_TASK (
                    ID_ TEXT PRIMARY KEY,
                    CASE_INSTANCE_ID_ TEXT NOT NULL,
                    CASE_DEFINITION_ID_ TEXT NOT NULL,
                    CASE_KEY_ TEXT NOT NULL,
                    STAGE_INSTANCE_ID_ TEXT,
                    STATE_ TEXT NOT NULL,
                    ACTIVATED_AT_ TEXT NOT NULL,
                    DATA_ TEXT NOT NULL
                )
            "#.to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_HUMAN_TASK".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_HUMAN_TASK (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    CASE_INSTANCE_ID_ VARCHAR(255) NOT NULL,
                    CASE_DEFINITION_ID_ VARCHAR(255) NOT NULL,
                    CASE_KEY_ VARCHAR(255) NOT NULL,
                    STAGE_INSTANCE_ID_ VARCHAR(255),
                    STATE_ VARCHAR(255) NOT NULL,
                    ACTIVATED_AT_ VARCHAR(255) NOT NULL,
                    DATA_ TEXT NOT NULL
                )
            "#.to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_HUMAN_TASK".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_HUMAN_TASK (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    CASE_INSTANCE_ID_ VARCHAR(255) NOT NULL,
                    CASE_DEFINITION_ID_ VARCHAR(255) NOT NULL,
                    CASE_KEY_ VARCHAR(255) NOT NULL,
                    STAGE_INSTANCE_ID_ VARCHAR(255),
                    STATE_ VARCHAR(255) NOT NULL,
                    ACTIVATED_AT_ VARCHAR(255) NOT NULL,
                    DATA_ TEXT NOT NULL
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#.to_string(),
        },
        // CMMN Case History table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_CASE_HISTORY".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_CASE_HISTORY (
                    CASE_INSTANCE_ID_ TEXT PRIMARY KEY,
                    CASE_DEFINITION_ID_ TEXT NOT NULL,
                    CASE_KEY_ TEXT NOT NULL,
                    TENANT_ID_ TEXT,
                    BUSINESS_KEY_ TEXT,
                    STATE_ TEXT NOT NULL,
                    STARTED_AT_ TEXT NOT NULL,
                    COMPLETED_AT_ TEXT,
                    DATA_ TEXT NOT NULL
                )
            "#.to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_CASE_HISTORY".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_CASE_HISTORY (
                    CASE_INSTANCE_ID_ VARCHAR(255) PRIMARY KEY,
                    CASE_DEFINITION_ID_ VARCHAR(255) NOT NULL,
                    CASE_KEY_ VARCHAR(255) NOT NULL,
                    TENANT_ID_ VARCHAR(255),
                    BUSINESS_KEY_ VARCHAR(255),
                    STATE_ VARCHAR(255) NOT NULL,
                    STARTED_AT_ VARCHAR(255) NOT NULL,
                    COMPLETED_AT_ VARCHAR(255),
                    DATA_ TEXT NOT NULL
                )
            "#.to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_CASE_HISTORY".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_CASE_HISTORY (
                    CASE_INSTANCE_ID_ VARCHAR(255) PRIMARY KEY,
                    CASE_DEFINITION_ID_ VARCHAR(255) NOT NULL,
                    CASE_KEY_ VARCHAR(255) NOT NULL,
                    TENANT_ID_ VARCHAR(255),
                    BUSINESS_KEY_ VARCHAR(255),
                    STATE_ VARCHAR(255) NOT NULL,
                    STARTED_AT_ VARCHAR(255) NOT NULL,
                    COMPLETED_AT_ VARCHAR(255),
                    DATA_ TEXT NOT NULL
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#.to_string(),
        },
        // CMMN Human Task History table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_HUMAN_TASK_HISTORY".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_HUMAN_TASK_HISTORY (
                    TASK_ID_ TEXT PRIMARY KEY,
                    CASE_INSTANCE_ID_ TEXT NOT NULL,
                    CASE_DEFINITION_ID_ TEXT NOT NULL,
                    CASE_KEY_ TEXT NOT NULL,
                    STAGE_INSTANCE_ID_ TEXT,
                    STATE_ TEXT NOT NULL,
                    ACTIVATED_AT_ TEXT NOT NULL,
                    COMPLETED_AT_ TEXT,
                    DATA_ TEXT NOT NULL
                )
            "#.to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_HUMAN_TASK_HISTORY".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_HUMAN_TASK_HISTORY (
                    TASK_ID_ VARCHAR(255) PRIMARY KEY,
                    CASE_INSTANCE_ID_ VARCHAR(255) NOT NULL,
                    CASE_DEFINITION_ID_ VARCHAR(255) NOT NULL,
                    CASE_KEY_ VARCHAR(255) NOT NULL,
                    STAGE_INSTANCE_ID_ VARCHAR(255),
                    STATE_ VARCHAR(255) NOT NULL,
                    ACTIVATED_AT_ VARCHAR(255) NOT NULL,
                    COMPLETED_AT_ VARCHAR(255),
                    DATA_ TEXT NOT NULL
                )
            "#.to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_HUMAN_TASK_HISTORY".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_HUMAN_TASK_HISTORY (
                    TASK_ID_ VARCHAR(255) PRIMARY KEY,
                    CASE_INSTANCE_ID_ VARCHAR(255) NOT NULL,
                    CASE_DEFINITION_ID_ VARCHAR(255) NOT NULL,
                    CASE_KEY_ VARCHAR(255) NOT NULL,
                    STAGE_INSTANCE_ID_ VARCHAR(255),
                    STATE_ VARCHAR(255) NOT NULL,
                    ACTIVATED_AT_ VARCHAR(255) NOT NULL,
                    COMPLETED_AT_ VARCHAR(255),
                    DATA_ TEXT NOT NULL
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#.to_string(),
        },
        // CMMN Milestone History table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_MILESTONE_HISTORY".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_MILESTONE_HISTORY (
                    ID_ TEXT PRIMARY KEY,
                    CASE_INSTANCE_ID_ TEXT NOT NULL,
                    CASE_DEFINITION_ID_ TEXT NOT NULL,
                    CASE_KEY_ TEXT NOT NULL,
                    MILESTONE_ID_ TEXT NOT NULL,
                    TIME_ TEXT NOT NULL,
                    DATA_ TEXT NOT NULL
                )
            "#.to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_MILESTONE_HISTORY".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_MILESTONE_HISTORY (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    CASE_INSTANCE_ID_ VARCHAR(255) NOT NULL,
                    CASE_DEFINITION_ID_ VARCHAR(255) NOT NULL,
                    CASE_KEY_ VARCHAR(255) NOT NULL,
                    MILESTONE_ID_ VARCHAR(255) NOT NULL,
                    TIME_ VARCHAR(255) NOT NULL,
                    DATA_ TEXT NOT NULL
                )
            "#.to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_MILESTONE_HISTORY".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_MILESTONE_HISTORY (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    CASE_INSTANCE_ID_ VARCHAR(255) NOT NULL,
                    CASE_DEFINITION_ID_ VARCHAR(255) NOT NULL,
                    CASE_KEY_ VARCHAR(255) NOT NULL,
                    MILESTONE_ID_ VARCHAR(255) NOT NULL,
                    TIME_ VARCHAR(255) NOT NULL,
                    DATA_ TEXT NOT NULL
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#.to_string(),
        },
        // CMMN Identity Link table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_IDENTITY_LINK".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_IDENTITY_LINK (
                    ID_ TEXT PRIMARY KEY,
                    SCOPE_TYPE_ TEXT NOT NULL,
                    SCOPE_ID_ TEXT NOT NULL,
                    LINK_TYPE_ TEXT NOT NULL,
                    USER_ID_ TEXT,
                    GROUP_ID_ TEXT,
                    DATA_ TEXT NOT NULL
                )
            "#.to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_IDENTITY_LINK".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_IDENTITY_LINK (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    SCOPE_TYPE_ VARCHAR(255) NOT NULL,
                    SCOPE_ID_ VARCHAR(255) NOT NULL,
                    LINK_TYPE_ VARCHAR(255) NOT NULL,
                    USER_ID_ VARCHAR(255),
                    GROUP_ID_ VARCHAR(255),
                    DATA_ TEXT NOT NULL
                )
            "#.to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_IDENTITY_LINK".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_IDENTITY_LINK (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    SCOPE_TYPE_ VARCHAR(255) NOT NULL,
                    SCOPE_ID_ VARCHAR(255) NOT NULL,
                    LINK_TYPE_ VARCHAR(255) NOT NULL,
                    USER_ID_ VARCHAR(255),
                    GROUP_ID_ VARCHAR(255),
                    DATA_ TEXT NOT NULL
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#.to_string(),
        },
        // CMMN Job table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_JOB".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_JOB (
                    ID_ TEXT PRIMARY KEY,
                    FAMILY_ TEXT NOT NULL,
                    STATE_ TEXT NOT NULL,
                    SCOPE_ID_ TEXT,
                    SUB_SCOPE_ID_ TEXT,
                    SCOPE_DEFINITION_ID_ TEXT,
                    ELEMENT_ID_ TEXT,
                    TENANT_ID_ TEXT,
                    DUE_DATE_ TEXT,
                    LOCK_OWNER_ TEXT,
                    RETRIES_ INTEGER NOT NULL,
                    EXCEPTION_MESSAGE_ TEXT,
                    EXCEPTION_STACKTRACE_ TEXT,
                    CREATED_AT_ INTEGER NOT NULL,
                    DATA_ TEXT NOT NULL
                )
            "#.to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_JOB".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_JOB (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    FAMILY_ VARCHAR(255) NOT NULL,
                    STATE_ VARCHAR(255) NOT NULL,
                    SCOPE_ID_ VARCHAR(255),
                    SUB_SCOPE_ID_ VARCHAR(255),
                    SCOPE_DEFINITION_ID_ VARCHAR(255),
                    ELEMENT_ID_ VARCHAR(255),
                    TENANT_ID_ VARCHAR(255),
                    DUE_DATE_ VARCHAR(255),
                    LOCK_OWNER_ VARCHAR(255),
                    RETRIES_ INTEGER NOT NULL,
                    EXCEPTION_MESSAGE_ TEXT,
                    EXCEPTION_STACKTRACE_ TEXT,
                    CREATED_AT_ BIGINT NOT NULL,
                    DATA_ TEXT NOT NULL
                )
            "#.to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_JOB".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_JOB (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    FAMILY_ VARCHAR(255) NOT NULL,
                    STATE_ VARCHAR(255) NOT NULL,
                    SCOPE_ID_ VARCHAR(255),
                    SUB_SCOPE_ID_ VARCHAR(255),
                    SCOPE_DEFINITION_ID_ VARCHAR(255),
                    ELEMENT_ID_ VARCHAR(255),
                    TENANT_ID_ VARCHAR(255),
                    DUE_DATE_ VARCHAR(255),
                    LOCK_OWNER_ VARCHAR(255),
                    RETRIES_ INTEGER NOT NULL,
                    EXCEPTION_MESSAGE_ TEXT,
                    EXCEPTION_STACKTRACE_ TEXT,
                    CREATED_AT_ BIGINT NOT NULL,
                    DATA_ TEXT NOT NULL
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#.to_string(),
        },
        // CMMN Event Subscription table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_EVENT_SUBSCRIPTION".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_EVENT_SUBSCRIPTION (
                    ID_ TEXT PRIMARY KEY,
                    EVENT_TYPE_ TEXT NOT NULL,
                    EVENT_NAME_ TEXT,
                    ACTIVITY_ID_ TEXT,
                    CASE_INSTANCE_ID_ TEXT,
                    CASE_DEFINITION_ID_ TEXT,
                    PLAN_ITEM_INSTANCE_ID_ TEXT,
                    TENANT_ID_ TEXT,
                    CONFIGURATION_ TEXT,
                    CREATED_AT_ INTEGER NOT NULL,
                    DATA_ TEXT NOT NULL
                )
            "#.to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_EVENT_SUBSCRIPTION".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_EVENT_SUBSCRIPTION (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    EVENT_TYPE_ VARCHAR(255) NOT NULL,
                    EVENT_NAME_ VARCHAR(255),
                    ACTIVITY_ID_ VARCHAR(255),
                    CASE_INSTANCE_ID_ VARCHAR(255),
                    CASE_DEFINITION_ID_ VARCHAR(255),
                    PLAN_ITEM_INSTANCE_ID_ VARCHAR(255),
                    TENANT_ID_ VARCHAR(255),
                    CONFIGURATION_ VARCHAR(255),
                    CREATED_AT_ BIGINT NOT NULL,
                    DATA_ TEXT NOT NULL
                )
            "#.to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_EVENT_SUBSCRIPTION".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_EVENT_SUBSCRIPTION (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    EVENT_TYPE_ VARCHAR(255) NOT NULL,
                    EVENT_NAME_ VARCHAR(255),
                    ACTIVITY_ID_ VARCHAR(255),
                    CASE_INSTANCE_ID_ VARCHAR(255),
                    CASE_DEFINITION_ID_ VARCHAR(255),
                    PLAN_ITEM_INSTANCE_ID_ VARCHAR(255),
                    TENANT_ID_ VARCHAR(255),
                    CONFIGURATION_ VARCHAR(255),
                    CREATED_AT_ BIGINT NOT NULL,
                    DATA_ TEXT NOT NULL
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#.to_string(),
        },
        // CMMN Task Instance Association table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_TASK_INSTANCE_ASSOCIATION".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_TASK_INSTANCE_ASSOCIATION (
                    ID_ TEXT PRIMARY KEY,
                    KIND_ TEXT NOT NULL,
                    STATE_ TEXT NOT NULL,
                    CASE_INSTANCE_ID_ TEXT NOT NULL,
                    CASE_DEFINITION_ID_ TEXT NOT NULL,
                    CASE_KEY_ TEXT NOT NULL,
                    STAGE_INSTANCE_ID_ TEXT,
                    PLAN_ITEM_ID_ TEXT NOT NULL,
                    TASK_DEFINITION_ID_ TEXT NOT NULL,
                    CHILD_DEFINITION_KEY_ TEXT NOT NULL,
                    CHILD_INSTANCE_ID_ TEXT NOT NULL,
                    CREATED_AT_ INTEGER NOT NULL,
                    COMPLETED_AT_ TEXT,
                    FAILURE_MESSAGE_ TEXT,
                    DATA_ TEXT NOT NULL
                )
            "#.to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_TASK_INSTANCE_ASSOCIATION".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_TASK_INSTANCE_ASSOCIATION (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    KIND_ VARCHAR(255) NOT NULL,
                    STATE_ VARCHAR(255) NOT NULL,
                    CASE_INSTANCE_ID_ VARCHAR(255) NOT NULL,
                    CASE_DEFINITION_ID_ VARCHAR(255) NOT NULL,
                    CASE_KEY_ VARCHAR(255) NOT NULL,
                    STAGE_INSTANCE_ID_ VARCHAR(255),
                    PLAN_ITEM_ID_ VARCHAR(255) NOT NULL,
                    TASK_DEFINITION_ID_ VARCHAR(255) NOT NULL,
                    CHILD_DEFINITION_KEY_ VARCHAR(255) NOT NULL,
                    CHILD_INSTANCE_ID_ VARCHAR(255) NOT NULL,
                    CREATED_AT_ BIGINT NOT NULL,
                    COMPLETED_AT_ VARCHAR(255),
                    FAILURE_MESSAGE_ TEXT,
                    DATA_ TEXT NOT NULL
                )
            "#.to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_TASK_INSTANCE_ASSOCIATION".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_TASK_INSTANCE_ASSOCIATION (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    KIND_ VARCHAR(255) NOT NULL,
                    STATE_ VARCHAR(255) NOT NULL,
                    CASE_INSTANCE_ID_ VARCHAR(255) NOT NULL,
                    CASE_DEFINITION_ID_ VARCHAR(255) NOT NULL,
                    CASE_KEY_ VARCHAR(255) NOT NULL,
                    STAGE_INSTANCE_ID_ VARCHAR(255),
                    PLAN_ITEM_ID_ VARCHAR(255) NOT NULL,
                    TASK_DEFINITION_ID_ VARCHAR(255) NOT NULL,
                    CHILD_DEFINITION_KEY_ VARCHAR(255) NOT NULL,
                    CHILD_INSTANCE_ID_ VARCHAR(255) NOT NULL,
                    CREATED_AT_ BIGINT NOT NULL,
                    COMPLETED_AT_ VARCHAR(255),
                    FAILURE_MESSAGE_ TEXT,
                    DATA_ TEXT NOT NULL
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#.to_string(),
        },
        // CMMN Plan Item Event table
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_PLAN_ITEM_EVENT".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_PLAN_ITEM_EVENT (
                    ID_ TEXT PRIMARY KEY,
                    CASE_INSTANCE_ID_ TEXT NOT NULL,
                    PLAN_ITEM_ID_ TEXT NOT NULL,
                    STANDARD_EVENT_ TEXT NOT NULL,
                    OCCURRED_AT_ INTEGER NOT NULL
                )
            "#.to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_PLAN_ITEM_EVENT".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_PLAN_ITEM_EVENT (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    CASE_INSTANCE_ID_ VARCHAR(255) NOT NULL,
                    PLAN_ITEM_ID_ VARCHAR(255) NOT NULL,
                    STANDARD_EVENT_ VARCHAR(255) NOT NULL,
                    OCCURRED_AT_ BIGINT NOT NULL
                )
            "#.to_string(),
        },
        SchemaScript {
            version: "7.0.0".to_string(),
            component: "ACT_CMMN_PLAN_ITEM_EVENT".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_PLAN_ITEM_EVENT (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    CASE_INSTANCE_ID_ VARCHAR(255) NOT NULL,
                    PLAN_ITEM_ID_ VARCHAR(255) NOT NULL,
                    STANDARD_EVENT_ VARCHAR(255) NOT NULL,
                    OCCURRED_AT_ BIGINT NOT NULL
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#.to_string(),
        },
        // P116: unified runtime plan item instance table (Java ACT_CMMN_RU_PLAN_ITEM_INST,
        // flowable.h2.create.cmmn.sql:84-124). Stage / milestone / event listener rows are
        // mirrored here so the unified plan-item-instance query reads one table; human-task
        // rows stay backed by ACT_CMMN_HUMAN_TASK.
        SchemaScript {
            version: "7.1.0".to_string(),
            component: "ACT_CMMN_RU_PLAN_ITEM_INST".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_RU_PLAN_ITEM_INST (
                    ID_ TEXT PRIMARY KEY,
                    CASE_DEF_ID_ TEXT NOT NULL,
                    CASE_INST_ID_ TEXT NOT NULL,
                    STAGE_INST_ID_ TEXT,
                    ELEMENT_ID_ TEXT NOT NULL,
                    ITEM_DEFINITION_ID_ TEXT NOT NULL,
                    ITEM_DEFINITION_TYPE_ TEXT NOT NULL,
                    NAME_ TEXT NOT NULL,
                    STATE_ TEXT NOT NULL,
                    CREATE_TIME_ TEXT NOT NULL,
                    ENDED_TIME_ TEXT,
                    OCCURRED_TIME_ TEXT,
                    ASSIGNEE_ TEXT,
                    TENANT_ID_ TEXT,
                    DATA_ TEXT NOT NULL
                )
            "#.to_string(),
        },
        SchemaScript {
            version: "7.1.0".to_string(),
            component: "ACT_CMMN_RU_PLAN_ITEM_INST".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_RU_PLAN_ITEM_INST (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    CASE_DEF_ID_ VARCHAR(255) NOT NULL,
                    CASE_INST_ID_ VARCHAR(255) NOT NULL,
                    STAGE_INST_ID_ VARCHAR(255),
                    ELEMENT_ID_ VARCHAR(255) NOT NULL,
                    ITEM_DEFINITION_ID_ VARCHAR(255) NOT NULL,
                    ITEM_DEFINITION_TYPE_ VARCHAR(255) NOT NULL,
                    NAME_ VARCHAR(255) NOT NULL,
                    STATE_ VARCHAR(255) NOT NULL,
                    CREATE_TIME_ VARCHAR(255) NOT NULL,
                    ENDED_TIME_ VARCHAR(255),
                    OCCURRED_TIME_ VARCHAR(255),
                    ASSIGNEE_ VARCHAR(255),
                    TENANT_ID_ VARCHAR(255),
                    DATA_ TEXT NOT NULL
                )
            "#.to_string(),
        },
        SchemaScript {
            version: "7.1.0".to_string(),
            component: "ACT_CMMN_RU_PLAN_ITEM_INST".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_CMMN_RU_PLAN_ITEM_INST (
                    ID_ VARCHAR(255) PRIMARY KEY,
                    CASE_DEF_ID_ VARCHAR(255) NOT NULL,
                    CASE_INST_ID_ VARCHAR(255) NOT NULL,
                    STAGE_INST_ID_ VARCHAR(255),
                    ELEMENT_ID_ VARCHAR(255) NOT NULL,
                    ITEM_DEFINITION_ID_ VARCHAR(255) NOT NULL,
                    ITEM_DEFINITION_TYPE_ VARCHAR(255) NOT NULL,
                    NAME_ VARCHAR(255) NOT NULL,
                    STATE_ VARCHAR(255) NOT NULL,
                    CREATE_TIME_ VARCHAR(255) NOT NULL,
                    ENDED_TIME_ VARCHAR(255),
                    OCCURRED_TIME_ VARCHAR(255),
                    ASSIGNEE_ VARCHAR(255),
                    TENANT_ID_ VARCHAR(255),
                    DATA_ TEXT NOT NULL
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
            "#.to_string(),
        },
        SchemaScript {
            version: "7.1.0".to_string(),
            component: "ACT_CMMN_DEPLOYMENT".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                ALTER TABLE ACT_CMMN_DEPLOYMENT ADD COLUMN CATEGORY_ TEXT;
                ALTER TABLE ACT_CMMN_DEPLOYMENT ADD COLUMN KEY_ TEXT;
                ALTER TABLE ACT_CMMN_DEPLOYMENT ADD COLUMN PARENT_DEPLOYMENT_ID_ TEXT;
                ALTER TABLE ACT_CMMN_CASE_DEFINITION ADD COLUMN CATEGORY_ TEXT;
                ALTER TABLE ACT_CMMN_CASE_DEFINITION ADD COLUMN DIAGRAM_RESOURCE_NAME_ TEXT;
                CREATE INDEX IF NOT EXISTS IDX_CMMN_DEPLOYMENT_KEY ON ACT_CMMN_DEPLOYMENT (KEY_);
                CREATE INDEX IF NOT EXISTS IDX_CMMN_DEPLOYMENT_CATEGORY ON ACT_CMMN_DEPLOYMENT (CATEGORY_);
                CREATE INDEX IF NOT EXISTS IDX_CMMN_DEPLOYMENT_PARENT ON ACT_CMMN_DEPLOYMENT (PARENT_DEPLOYMENT_ID_);
                CREATE INDEX IF NOT EXISTS IDX_CMMN_CASE_DEFINITION_DEPLOYMENT ON ACT_CMMN_CASE_DEFINITION (DEPLOYMENT_ID_);
                CREATE INDEX IF NOT EXISTS IDX_CMMN_CASE_INSTANCE_DEFINITION ON ACT_CMMN_CASE_INSTANCE (CASE_DEFINITION_ID_);
                CREATE INDEX IF NOT EXISTS IDX_CMMN_JOB_SCOPE ON ACT_CMMN_JOB (SCOPE_ID_, SUB_SCOPE_ID_, SCOPE_DEFINITION_ID_);
                CREATE INDEX IF NOT EXISTS IDX_CMMN_CASE_DEFINITION_KEY_TENANT_VERSION ON ACT_CMMN_CASE_DEFINITION (CASE_KEY_, TENANT_ID_, VERSION_);
                CREATE INDEX IF NOT EXISTS IDX_CMMN_CASE_DEFINITION_CATEGORY ON ACT_CMMN_CASE_DEFINITION (CATEGORY_);
                CREATE INDEX IF NOT EXISTS IDX_CMMN_CASE_HISTORY_DEFINITION_INSTANCE ON ACT_CMMN_CASE_HISTORY (CASE_DEFINITION_ID_, CASE_INSTANCE_ID_);
                CREATE INDEX IF NOT EXISTS IDX_CMMN_IDENTITY_LINK_SCOPES ON ACT_CMMN_IDENTITY_LINK (SCOPE_TYPE_, SCOPE_ID_);
                CREATE INDEX IF NOT EXISTS IDX_CMMN_EVENT_SUBSCRIPTION_SCOPES ON ACT_CMMN_EVENT_SUBSCRIPTION (CASE_DEFINITION_ID_, CASE_INSTANCE_ID_);
            "#.to_string(),
        },
        SchemaScript {
            version: "7.1.0".to_string(),
            component: "ACT_CMMN_DEPLOYMENT".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                ALTER TABLE ACT_CMMN_DEPLOYMENT ADD COLUMN CATEGORY_ VARCHAR(255);
                ALTER TABLE ACT_CMMN_DEPLOYMENT ADD COLUMN KEY_ VARCHAR(255);
                ALTER TABLE ACT_CMMN_DEPLOYMENT ADD COLUMN PARENT_DEPLOYMENT_ID_ VARCHAR(255);
                ALTER TABLE ACT_CMMN_CASE_DEFINITION ADD COLUMN CATEGORY_ VARCHAR(255);
                ALTER TABLE ACT_CMMN_CASE_DEFINITION ADD COLUMN DIAGRAM_RESOURCE_NAME_ VARCHAR(255);
                CREATE INDEX IF NOT EXISTS IDX_CMMN_DEPLOYMENT_KEY ON ACT_CMMN_DEPLOYMENT (KEY_);
                CREATE INDEX IF NOT EXISTS IDX_CMMN_DEPLOYMENT_CATEGORY ON ACT_CMMN_DEPLOYMENT (CATEGORY_);
                CREATE INDEX IF NOT EXISTS IDX_CMMN_DEPLOYMENT_PARENT ON ACT_CMMN_DEPLOYMENT (PARENT_DEPLOYMENT_ID_);
                CREATE INDEX IF NOT EXISTS IDX_CMMN_CASE_DEFINITION_DEPLOYMENT ON ACT_CMMN_CASE_DEFINITION (DEPLOYMENT_ID_);
                CREATE INDEX IF NOT EXISTS IDX_CMMN_CASE_INSTANCE_DEFINITION ON ACT_CMMN_CASE_INSTANCE (CASE_DEFINITION_ID_);
                CREATE INDEX IF NOT EXISTS IDX_CMMN_JOB_SCOPE ON ACT_CMMN_JOB (SCOPE_ID_, SUB_SCOPE_ID_, SCOPE_DEFINITION_ID_);
                CREATE INDEX IF NOT EXISTS IDX_CMMN_CASE_DEFINITION_KEY_TENANT_VERSION ON ACT_CMMN_CASE_DEFINITION (CASE_KEY_, TENANT_ID_, VERSION_);
                CREATE INDEX IF NOT EXISTS IDX_CMMN_CASE_DEFINITION_CATEGORY ON ACT_CMMN_CASE_DEFINITION (CATEGORY_);
                CREATE INDEX IF NOT EXISTS IDX_CMMN_CASE_HISTORY_DEFINITION_INSTANCE ON ACT_CMMN_CASE_HISTORY (CASE_DEFINITION_ID_, CASE_INSTANCE_ID_);
                CREATE INDEX IF NOT EXISTS IDX_CMMN_IDENTITY_LINK_SCOPES ON ACT_CMMN_IDENTITY_LINK (SCOPE_TYPE_, SCOPE_ID_);
                CREATE INDEX IF NOT EXISTS IDX_CMMN_EVENT_SUBSCRIPTION_SCOPES ON ACT_CMMN_EVENT_SUBSCRIPTION (CASE_DEFINITION_ID_, CASE_INSTANCE_ID_);
            "#.to_string(),
        },
        SchemaScript {
            version: "7.1.0".to_string(),
            component: "ACT_CMMN_DEPLOYMENT".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                ALTER TABLE ACT_CMMN_DEPLOYMENT ADD COLUMN CATEGORY_ VARCHAR(255);
                ALTER TABLE ACT_CMMN_DEPLOYMENT ADD COLUMN KEY_ VARCHAR(255);
                ALTER TABLE ACT_CMMN_DEPLOYMENT ADD COLUMN PARENT_DEPLOYMENT_ID_ VARCHAR(255);
                ALTER TABLE ACT_CMMN_CASE_DEFINITION ADD COLUMN CATEGORY_ VARCHAR(255);
                ALTER TABLE ACT_CMMN_CASE_DEFINITION ADD COLUMN DIAGRAM_RESOURCE_NAME_ VARCHAR(255);
                CREATE INDEX IDX_CMMN_DEPLOYMENT_KEY ON ACT_CMMN_DEPLOYMENT (KEY_);
                CREATE INDEX IDX_CMMN_DEPLOYMENT_CATEGORY ON ACT_CMMN_DEPLOYMENT (CATEGORY_);
                CREATE INDEX IDX_CMMN_DEPLOYMENT_PARENT ON ACT_CMMN_DEPLOYMENT (PARENT_DEPLOYMENT_ID_);
                CREATE INDEX IDX_CMMN_CASE_DEFINITION_DEPLOYMENT ON ACT_CMMN_CASE_DEFINITION (DEPLOYMENT_ID_);
                CREATE INDEX IDX_CMMN_CASE_INSTANCE_DEFINITION ON ACT_CMMN_CASE_INSTANCE (CASE_DEFINITION_ID_);
                CREATE INDEX IDX_CMMN_JOB_SCOPE ON ACT_CMMN_JOB (SCOPE_ID_, SUB_SCOPE_ID_, SCOPE_DEFINITION_ID_);
                CREATE INDEX IDX_CMMN_CASE_DEFINITION_KEY_TENANT_VERSION ON ACT_CMMN_CASE_DEFINITION (CASE_KEY_, TENANT_ID_, VERSION_);
                CREATE INDEX IDX_CMMN_CASE_DEFINITION_CATEGORY ON ACT_CMMN_CASE_DEFINITION (CATEGORY_);
                CREATE INDEX IDX_CMMN_CASE_HISTORY_DEFINITION_INSTANCE ON ACT_CMMN_CASE_HISTORY (CASE_DEFINITION_ID_, CASE_INSTANCE_ID_);
                CREATE INDEX IDX_CMMN_IDENTITY_LINK_SCOPES ON ACT_CMMN_IDENTITY_LINK (SCOPE_TYPE_, SCOPE_ID_);
                CREATE INDEX IDX_CMMN_EVENT_SUBSCRIPTION_SCOPES ON ACT_CMMN_EVENT_SUBSCRIPTION (CASE_DEFINITION_ID_, CASE_INSTANCE_ID_);
            "#.to_string(),
        },
        // 7.1.1: widen dual-write id columns so `{key}:{version}:{uuid}` process
        // definition ids (often >64 chars) fit on PG/MySQL.
        SchemaScript {
            version: "7.1.1".to_string(),
            component: "ACT_RE_PROCDEF".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                ALTER TABLE ACT_RE_PROCDEF ALTER COLUMN ID_ TYPE VARCHAR(255);
                ALTER TABLE ACT_RE_PROCDEF ALTER COLUMN DEPLOYMENT_ID_ TYPE VARCHAR(255);
                ALTER TABLE ACT_GE_BYTEARRAY ALTER COLUMN ID_ TYPE VARCHAR(255);
                ALTER TABLE ACT_GE_BYTEARRAY ALTER COLUMN DEPLOYMENT_ID_ TYPE VARCHAR(255);
                ALTER TABLE ACT_RE_DEPLOYMENT ALTER COLUMN ID_ TYPE VARCHAR(255);
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.1.1".to_string(),
            component: "ACT_RE_PROCDEF".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                ALTER TABLE ACT_RE_PROCDEF MODIFY ID_ VARCHAR(255);
                ALTER TABLE ACT_RE_PROCDEF MODIFY DEPLOYMENT_ID_ VARCHAR(255);
                ALTER TABLE ACT_GE_BYTEARRAY MODIFY ID_ VARCHAR(255);
                ALTER TABLE ACT_GE_BYTEARRAY MODIFY DEPLOYMENT_ID_ VARCHAR(255);
                ALTER TABLE ACT_RE_DEPLOYMENT MODIFY ID_ VARCHAR(255);
            "#
            .to_string(),
        },
        // SQLite already uses TEXT for id columns — no 7.1.1 sqlite script.
        // schema.version still advances past 7.1.1 via latest_version() which
        // considers scripts across all backends (chain tail is currently 7.1.2).

        // 7.1.2 / P77: ACT_HI_IDENTITYLINK — Java create SQL
        // flowable.postgres.all.create.sql:95-113 (and mysql/oracle equivalents).
        SchemaScript {
            version: "7.1.2".to_string(),
            component: "ACT_HI_IDENTITYLINK".to_string(),
            database_type: "sqlite".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_HI_IDENTITYLINK (
                    ID_ TEXT PRIMARY KEY,
                    GROUP_ID_ TEXT,
                    TYPE_ TEXT,
                    USER_ID_ TEXT,
                    TASK_ID_ TEXT,
                    CREATE_TIME_ INTEGER,
                    PROC_INST_ID_ TEXT,
                    SCOPE_ID_ TEXT,
                    SUB_SCOPE_ID_ TEXT,
                    SCOPE_TYPE_ TEXT,
                    SCOPE_DEFINITION_ID_ TEXT
                );
                CREATE INDEX IF NOT EXISTS ACT_IDX_HI_IDENT_LNK_USER ON ACT_HI_IDENTITYLINK(USER_ID_);
                CREATE INDEX IF NOT EXISTS ACT_IDX_HI_IDENT_LNK_SCOPE ON ACT_HI_IDENTITYLINK(SCOPE_ID_, SCOPE_TYPE_);
                CREATE INDEX IF NOT EXISTS ACT_IDX_HI_IDENT_LNK_SUB_SCOPE ON ACT_HI_IDENTITYLINK(SUB_SCOPE_ID_, SCOPE_TYPE_);
                CREATE INDEX IF NOT EXISTS ACT_IDX_HI_IDENT_LNK_SCOPE_DEF ON ACT_HI_IDENTITYLINK(SCOPE_DEFINITION_ID_, SCOPE_TYPE_);
                CREATE INDEX IF NOT EXISTS ACT_IDX_HI_IDENT_LNK_TASK ON ACT_HI_IDENTITYLINK(TASK_ID_);
                CREATE INDEX IF NOT EXISTS ACT_IDX_HI_IDENT_LNK_PROCINST ON ACT_HI_IDENTITYLINK(PROC_INST_ID_);
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.1.2".to_string(),
            component: "ACT_HI_IDENTITYLINK".to_string(),
            database_type: "postgres".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_HI_IDENTITYLINK (
                    ID_ varchar(64) PRIMARY KEY,
                    GROUP_ID_ varchar(255),
                    TYPE_ varchar(255),
                    USER_ID_ varchar(255),
                    TASK_ID_ varchar(64),
                    CREATE_TIME_ timestamp,
                    PROC_INST_ID_ varchar(64),
                    SCOPE_ID_ varchar(255),
                    SUB_SCOPE_ID_ varchar(255),
                    SCOPE_TYPE_ varchar(255),
                    SCOPE_DEFINITION_ID_ varchar(255)
                );
                CREATE INDEX IF NOT EXISTS ACT_IDX_HI_IDENT_LNK_USER ON ACT_HI_IDENTITYLINK(USER_ID_);
                CREATE INDEX IF NOT EXISTS ACT_IDX_HI_IDENT_LNK_SCOPE ON ACT_HI_IDENTITYLINK(SCOPE_ID_, SCOPE_TYPE_);
                CREATE INDEX IF NOT EXISTS ACT_IDX_HI_IDENT_LNK_SUB_SCOPE ON ACT_HI_IDENTITYLINK(SUB_SCOPE_ID_, SCOPE_TYPE_);
                CREATE INDEX IF NOT EXISTS ACT_IDX_HI_IDENT_LNK_SCOPE_DEF ON ACT_HI_IDENTITYLINK(SCOPE_DEFINITION_ID_, SCOPE_TYPE_);
                CREATE INDEX IF NOT EXISTS ACT_IDX_HI_IDENT_LNK_TASK ON ACT_HI_IDENTITYLINK(TASK_ID_);
                CREATE INDEX IF NOT EXISTS ACT_IDX_HI_IDENT_LNK_PROCINST ON ACT_HI_IDENTITYLINK(PROC_INST_ID_);
            "#
            .to_string(),
        },
        SchemaScript {
            version: "7.1.2".to_string(),
            component: "ACT_HI_IDENTITYLINK".to_string(),
            database_type: "mysql".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS ACT_HI_IDENTITYLINK (
                    ID_ varchar(64) PRIMARY KEY,
                    GROUP_ID_ varchar(255),
                    TYPE_ varchar(255),
                    USER_ID_ varchar(255),
                    TASK_ID_ varchar(64),
                    CREATE_TIME_ datetime(3),
                    PROC_INST_ID_ varchar(64),
                    SCOPE_ID_ varchar(255),
                    SUB_SCOPE_ID_ varchar(255),
                    SCOPE_TYPE_ varchar(255),
                    SCOPE_DEFINITION_ID_ varchar(255)
                ) ENGINE=InnoDB DEFAULT CHARSET=utf8 COLLATE utf8_bin;
                CREATE INDEX ACT_IDX_HI_IDENT_LNK_USER ON ACT_HI_IDENTITYLINK(USER_ID_);
                CREATE INDEX ACT_IDX_HI_IDENT_LNK_SCOPE ON ACT_HI_IDENTITYLINK(SCOPE_ID_, SCOPE_TYPE_);
                CREATE INDEX ACT_IDX_HI_IDENT_LNK_SUB_SCOPE ON ACT_HI_IDENTITYLINK(SUB_SCOPE_ID_, SCOPE_TYPE_);
                CREATE INDEX ACT_IDX_HI_IDENT_LNK_SCOPE_DEF ON ACT_HI_IDENTITYLINK(SCOPE_DEFINITION_ID_, SCOPE_TYPE_);
                CREATE INDEX ACT_IDX_HI_IDENT_LNK_TASK ON ACT_HI_IDENTITYLINK(TASK_ID_);
                CREATE INDEX ACT_IDX_HI_IDENT_LNK_PROCINST ON ACT_HI_IDENTITYLINK(PROC_INST_ID_);
            "#
            .to_string(),
        },
    ]
}

pub fn get_all_scripts() -> Vec<SchemaScript> {
    let mut scripts = get_common_scripts();
    scripts.extend(get_engine_scripts());
    scripts.extend(get_dmn_scripts());
    scripts.extend(get_app_scripts());
    scripts.extend(get_cmmn_scripts());
    scripts
}
