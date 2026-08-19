#![allow(clippy::field_reassign_with_default)]
#![allow(clippy::redundant_guards)]
#![allow(clippy::nonminimal_bool)]
#![allow(clippy::if_same_then_else)]
#![allow(clippy::too_many_arguments)]

use flowable_bpmn_model::constants::*;
use flowable_bpmn_model::model::*;
use flowable_engine_common::FlowableError;
use indexmap::IndexMap;
use quick_xml::events::BytesStart;
use quick_xml::events::Event as XmlEvent;
use quick_xml::reader::Reader;
use serde_json::Value;
use std::collections::HashMap;
use uuid::Uuid;

mod writer;

pub use writer::{BpmnXmlWriteError, BpmnXmlWriter, write_bpmn_model};

fn parse_comma_separated_id_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

/// Maximum XML element nesting depth accepted by the BPMN converter (M3).
/// Bounds every converter recursion (subprocess descent, flow-element map
/// population, canonical JSON normalization) against hostile deeply-nested
/// documents; far beyond anything a legitimate process definition reaches.
const MAX_XML_NESTING_DEPTH: usize = 512;

/// Java `ValuedDataObjectXMLConverter` + type-specific `setValue` subclasses:
/// convert the raw extension text into Long/Double/Boolean/Date/JSON values at
/// convert time. Expressions like `${...}` are **not** evaluated — they remain
/// plain strings (parity with Java, which never runs EL on data object values).
fn convert_data_object_value(raw: &str, data_type: Option<&str>) -> Value {
    let trimmed = raw.trim();
    match data_type.map(|t| t.to_ascii_lowercase()).as_deref() {
        Some("int") | Some("integer") => trimmed
            .parse::<i64>()
            .map(|n| Value::Number(n.into()))
            .unwrap_or_else(|_| Value::String(raw.to_string())),
        Some("long") => trimmed
            .parse::<i64>()
            .map(|n| Value::Number(n.into()))
            .unwrap_or_else(|_| Value::String(raw.to_string())),
        Some("double") | Some("float") => trimmed
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number)
            .unwrap_or_else(|| Value::String(raw.to_string())),
        Some("boolean") | Some("bool") => trimmed
            .parse::<bool>()
            .map(Value::Bool)
            .unwrap_or_else(|_| Value::String(raw.to_string())),
        // Java DateDataObject stores a Date; we keep the ISO-8601 text so runtime
        // can round-trip without a Date type in serde_json.
        Some("datetime") | Some("date") => Value::String(raw.to_string()),
        Some("json") => {
            serde_json::from_str(trimmed).unwrap_or_else(|_| Value::String(raw.to_string()))
        }
        _ => Value::String(raw.to_string()),
    }
}

pub struct BpmnXMLConverter {}

impl Default for BpmnXMLConverter {
    fn default() -> Self {
        Self::new()
    }
}

impl BpmnXMLConverter {
    pub fn new() -> Self {
        Self {}
    }

    pub fn convert_to_bpmn_model(&self, xml: &str) -> BpmnModel {
        match self.try_convert_to_bpmn_model(xml) {
            Ok(model) => model,
            Err(error) => {
                tracing::warn!("{error}; returning empty BPMN model for legacy caller");
                BpmnModel::default()
            }
        }
    }

    pub fn try_convert_to_bpmn_model(&self, xml: &str) -> Result<BpmnModel, FlowableError> {
        self.validate_well_formed_xml(xml)?;

        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(true);

        let mut model = BpmnModel::default();
        let mut buf = Vec::new();

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(XmlEvent::Start(ref e)) => {
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), &reader);
                    if local_name == ELEMENT_DEFINITIONS {
                        self.parse_definitions(e, &mut reader, &mut model);
                    }
                }
                Ok(XmlEvent::Eof) => break,
                Err(e) => {
                    return Err(FlowableError::InvalidBpmnXml {
                        position: reader.buffer_position(),
                        message: e.to_string(),
                    });
                }
                _ => (),
            }
            buf.clear();
        }

        // Post-processing
        let errors = model.errors.clone();
        for process in &mut model.processes {
            self.post_process_process(process, &errors);
        }

        if let Some(first_process) = model.processes.first() {
            model.main_process = Some(first_process.clone());
        }

        Ok(model)
    }

    fn validate_well_formed_xml(&self, xml: &str) -> Result<(), FlowableError> {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(true);
        let mut buf = Vec::new();
        let mut open_elements: Vec<Vec<u8>> = Vec::new();

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(XmlEvent::Start(e)) => {
                    open_elements.push(e.local_name().as_ref().to_vec());
                    // M3: cap XML nesting so adversarial documents cannot push
                    // converter recursion (subprocess descent, element maps)
                    // into stack overflow. Depth beyond any legitimate BPMN.
                    if open_elements.len() > MAX_XML_NESTING_DEPTH {
                        return Err(FlowableError::InvalidBpmnXml {
                            position: reader.buffer_position(),
                            message: format!(
                                "XML element nesting exceeds the limit of {} levels",
                                MAX_XML_NESTING_DEPTH
                            ),
                        });
                    }
                }
                Ok(XmlEvent::End(e)) => {
                    let Some(open) = open_elements.pop() else {
                        return Err(FlowableError::InvalidBpmnXml {
                            position: reader.buffer_position(),
                            message: "unexpected closing element".to_string(),
                        });
                    };
                    if open.as_slice() != e.local_name().as_ref() {
                        return Err(FlowableError::InvalidBpmnXml {
                            position: reader.buffer_position(),
                            message: "mismatched closing element".to_string(),
                        });
                    }
                }
                Ok(XmlEvent::Eof) => {
                    if open_elements.is_empty() {
                        break;
                    }
                    return Err(FlowableError::InvalidBpmnXml {
                        position: reader.buffer_position(),
                        message: "unexpected end of file while parsing BPMN XML".to_string(),
                    });
                }
                Err(e) => {
                    return Err(FlowableError::InvalidBpmnXml {
                        position: reader.buffer_position(),
                        message: e.to_string(),
                    });
                }
                _ => {}
            }
            buf.clear();
        }

        Ok(())
    }

    pub fn to_canonical_contract_value(&self, model: &BpmnModel) -> Value {
        let mut value = serde_json::to_value(model).unwrap_or_else(|error| {
            tracing::error!(error = %error, "converter output should serialize to JSON");
            Value::Null
        });
        Self::normalize_canonical_contract_value(&mut value);
        value
    }

    fn normalize_canonical_contract_value(value: &mut Value) {
        match value {
            Value::Object(map) => {
                map.remove("elementType");
                map.remove("eventDefinitionType");
                map.remove("subProcessType");
                map.remove("artifactType");
                map.remove("errorRef");
                map.remove("isForCompensation");
                if matches!(map.get("artifactMap"), Some(Value::Object(entries)) if entries.is_empty())
                {
                    map.remove("artifactMap");
                }
                if matches!(map.get("artifacts"), Some(Value::Array(entries)) if entries.is_empty())
                {
                    map.remove("artifacts");
                }
                if matches!(map.get("value"), Some(Value::String(value)) if value.is_empty())
                    && (map.contains_key("itemSubjectRef") || map.contains_key("dataType"))
                {
                    map.insert("value".to_string(), Value::Null);
                }
                for child in map.values_mut() {
                    Self::normalize_canonical_contract_value(child);
                }
            }
            Value::Array(items) => {
                for item in items {
                    Self::normalize_canonical_contract_value(item);
                }
            }
            _ => {}
        }
    }

    fn ensure_id(&self, id: &mut Option<String>) {
        if id.is_none() || id.as_ref().unwrap().is_empty() {
            *id = Some(Uuid::new_v4().to_string());
        }
    }

    fn read_id_attribute(&self, e: &BytesStart, reader: &Reader<&[u8]>) -> Option<String> {
        for attr in e.attributes() {
            let Ok(attr) = attr else {
                continue;
            };
            if self.get_local_name_bytes(attr.key.as_ref(), reader) == ATTRIBUTE_ID {
                return Some(
                    attr.decode_and_unescape_value(reader.decoder())
                        .unwrap_or_default()
                        .into_owned(),
                );
            }
        }
        None
    }

    fn push_extension_attribute(
        &self,
        base_element: &mut BaseElement,
        local_key: String,
        raw_key: &str,
        value: String,
        namespaces: &IndexMap<String, String>,
    ) {
        let mut ext_attr = ExtensionAttribute::default();
        ext_attr.name = Some(local_key.clone());
        ext_attr.value = Some(value);
        if let Some(pos) = raw_key.find(':') {
            let prefix = &raw_key[..pos];
            ext_attr.namespace_prefix = Some(prefix.to_string());
            if let Some(ns) = namespaces.get(prefix) {
                ext_attr.namespace = Some(ns.clone());
            }
        }

        base_element
            .attributes
            .entry(local_key)
            .or_default()
            .push(ext_attr);
    }

    #[allow(clippy::collapsible_if)]
    fn parse_definitions(&self, e: &BytesStart, reader: &mut Reader<&[u8]>, model: &mut BpmnModel) {
        for attr in e.attributes() {
            let Ok(attr) = attr else {
                continue;
            };
            let key_bytes = attr.key.as_ref();
            let key = reader.decoder().decode(key_bytes).unwrap_or_default();
            let value = attr
                .decode_and_unescape_value(reader.decoder())
                .unwrap_or_default();

            if key == TARGET_NAMESPACE_ATTRIBUTE {
                model.target_namespace = Some(value.clone().into_owned());
                continue;
            }
            if key == ATTRIBUTE_EXPORTER {
                model.exporter = Some(value.clone().into_owned());
                continue;
            }
            if key == ATTRIBUTE_EXPORTER_VERSION {
                model.exporter_version = Some(value.clone().into_owned());
                continue;
            }
            if key == TYPE_LANGUAGE_ATTRIBUTE || key == EXPRESSION_LANGUAGE_ATTRIBUTE {
                continue;
            }
            if key == "xmlns" || key.starts_with("xmlns:") {
                let prefix = if key == "xmlns" {
                    "".to_string()
                } else {
                    key[6..].to_string()
                };
                if !prefix.is_empty() {
                    model.namespaces.insert(prefix, value.into_owned());
                }
                continue;
            }

            let mut ext_attr = ExtensionAttribute::default();
            let local_key = self.get_local_name_bytes(key_bytes, reader);
            ext_attr.name = Some(local_key.clone());
            ext_attr.value = Some(value.into_owned());
            if let Some(pos) = key.find(':') {
                let prefix = &key[..pos];
                ext_attr.namespace_prefix = Some(prefix.to_string());
                if let Some(ns) = model.namespaces.get(prefix) {
                    ext_attr.namespace = Some(ns.clone());
                }
            }
            model
                .definitions_attributes
                .entry(local_key)
                .or_default()
                .push(ext_attr);
        }

        let mut buf = Vec::new();
        while let Ok(event) = reader.read_event_into(&mut buf) {
            match event {
                XmlEvent::Start(ref e) | XmlEvent::Empty(ref e) => {
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == "collaboration" {
                        self.parse_collaboration(e, reader, model);
                    } else if local_name == "message" {
                        let message = self.parse_message(
                            e,
                            reader,
                            model,
                            matches!(event, XmlEvent::Empty(_)),
                        );
                        model.messages.push(message);
                    } else if local_name == "signal" {
                        let signal =
                            self.parse_signal(e, reader, matches!(event, XmlEvent::Empty(_)));
                        model.signals.push(signal);
                    } else if local_name == ELEMENT_ESCALATION {
                        let escalation =
                            self.parse_escalation(e, reader, matches!(event, XmlEvent::Empty(_)));
                        model.escalations.push(escalation);
                    } else if local_name == ELEMENT_PROCESS {
                        let mut process = Process::default();
                        self.parse_process(e, reader, &mut process, model);
                        model.processes.push(process);
                    } else if local_name == "dataStore" {
                        let mut data_store = DataStore::default();
                        for attr in e.attributes() {
                            let Ok(attr) = attr else {
                                continue;
                            };
                            let key = self.get_local_name_bytes(attr.key.as_ref(), reader);
                            let value = attr
                                .decode_and_unescape_value(reader.decoder())
                                .unwrap_or_default();
                            if key == ATTRIBUTE_ID {
                                data_store.base_element.id = Some(value.into_owned());
                            } else if key == ATTRIBUTE_NAME {
                                data_store.name = Some(value.into_owned());
                            } else if key == ATTRIBUTE_ITEM_SUBJECT_REF {
                                data_store.item_subject_ref = Some(value.into_owned());
                            }
                        }
                        if let Some(ref id) = data_store.base_element.id {
                            model.data_stores.insert(id.clone(), data_store);
                        }
                    } else if local_name == ELEMENT_DI_DIAGRAM {
                        self.parse_di(e, reader, model);
                    } else if local_name == ELEMENT_ERROR {
                        let mut id = String::new();
                        let mut code = None;
                        for attr in e.attributes() {
                            let Ok(attr) = attr else {
                                continue;
                            };
                            let key = self.get_local_name_bytes(attr.key.as_ref(), reader);
                            let value = attr
                                .decode_and_unescape_value(reader.decoder())
                                .unwrap_or_default();
                            if key == ATTRIBUTE_ID {
                                id = value.into_owned();
                            } else if key == ATTRIBUTE_ERROR_CODE {
                                code = Some(value.into_owned());
                            }
                        }
                        if !id.is_empty() {
                            if let Some(c) = code {
                                model.errors.insert(id, c);
                            }
                        }
                    }
                }
                XmlEvent::End(ref e) => {
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == ELEMENT_DEFINITIONS {
                        break;
                    }
                }
                XmlEvent::Eof => break,
                _ => {}
            }
            buf.clear();
        }
    }

    fn parse_process(
        &self,
        e: &BytesStart,
        reader: &mut Reader<&[u8]>,
        process: &mut Process,
        model: &mut BpmnModel,
    ) {
        let offset = reader.buffer_position();
        let (row, col) = self.get_position(reader, offset as usize);
        process.base_element.xml_row_number = row;
        process.base_element.xml_column_number = col;

        for attr in e.attributes() {
            let Ok(attr) = attr else {
                continue;
            };
            let key_bytes = attr.key.as_ref();
            let key = reader.decoder().decode(key_bytes).unwrap_or_default();
            let local_key = self.get_local_name_bytes(key_bytes, reader);
            let value = attr
                .decode_and_unescape_value(reader.decoder())
                .unwrap_or_default();
            match local_key.as_str() {
                k if k == ATTRIBUTE_ID => process.base_element.id = Some(value.into_owned()),
                k if k == ATTRIBUTE_NAME => process.name = Some(value.into_owned()),
                k if k == ATTRIBUTE_PROCESS_EXECUTABLE => {
                    process.executable = value == ATTRIBUTE_VALUE_TRUE
                }
                _ => {
                    let mut ext_attr = ExtensionAttribute::default();
                    ext_attr.name = Some(local_key.clone());
                    ext_attr.value = Some(value.into_owned());
                    if let Some(pos) = key.find(':') {
                        let prefix = &key[..pos];
                        ext_attr.namespace_prefix = Some(prefix.to_string());
                        if let Some(ns) = model.namespaces.get(prefix) {
                            ext_attr.namespace = Some(ns.clone());
                        }
                    }
                    process
                        .base_element
                        .attributes
                        .entry(local_key)
                        .or_default()
                        .push(ext_attr);
                }
            }
        }
        self.ensure_id(&mut process.base_element.id);
        let process_namespaces = self.collect_namespaces_from_start(&model.namespaces, e, reader);

        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(XmlEvent::Start(ref inner_e)) => {
                    let local_name =
                        self.get_local_name_bytes(inner_e.local_name().as_ref(), reader);
                    if local_name == ELEMENT_DOCUMENTATION {
                        process.documentation =
                            Some(self.read_element_text(reader, inner_e.name()));
                    } else if local_name == ELEMENT_DATA_OBJECT
                        || local_name == ELEMENT_DATA_OBJECT_REFERENCE
                    {
                        let obj =
                            self.parse_data_object(inner_e, reader, false, &process_namespaces);
                        process.data_objects.push(obj.clone());
                        process
                            .flow_elements
                            .push(FlowElementEnum::ValuedDataObject(obj));
                    } else if local_name == "laneSet" {
                        self.parse_lane_set(inner_e, reader, process, &process_namespaces);
                    } else if local_name == "extensionElements" {
                        self.parse_extensions_into_process(
                            reader,
                            process,
                            inner_e,
                            &process_namespaces,
                        );
                    } else if let Some(artifact) =
                        self.parse_artifact(inner_e, reader, false, &process_namespaces)
                    {
                        if let ArtifactEnum::Association(association) = &artifact {
                            process.associations.push(association.clone());
                        }
                        process.artifacts.push(artifact);
                    } else if let Some(elem) =
                        self.parse_flow_element(inner_e, reader, model, false)
                    {
                        process.flow_elements.push(elem);
                    }
                }
                Ok(XmlEvent::Empty(ref inner_e)) => {
                    let local_name =
                        self.get_local_name_bytes(inner_e.local_name().as_ref(), reader);
                    if local_name == ELEMENT_DATA_OBJECT
                        || local_name == ELEMENT_DATA_OBJECT_REFERENCE
                    {
                        let obj =
                            self.parse_data_object(inner_e, reader, true, &process_namespaces);
                        process.data_objects.push(obj.clone());
                        process
                            .flow_elements
                            .push(FlowElementEnum::ValuedDataObject(obj));
                    } else if let Some(artifact) =
                        self.parse_artifact(inner_e, reader, true, &process_namespaces)
                    {
                        if let ArtifactEnum::Association(association) = &artifact {
                            process.associations.push(association.clone());
                        }
                        process.artifacts.push(artifact);
                    } else if let Some(elem) = self.parse_flow_element(inner_e, reader, model, true)
                    {
                        process.flow_elements.push(elem);
                    }
                }
                Ok(XmlEvent::End(ref inner_e)) => {
                    let local_name =
                        self.get_local_name_bytes(inner_e.local_name().as_ref(), reader);
                    if local_name == ELEMENT_PROCESS {
                        break;
                    }
                }
                Ok(XmlEvent::Eof) => break,
                _ => {}
            }
            buf.clear();
        }
    }

    fn parse_message(
        &self,
        e: &BytesStart,
        reader: &mut Reader<&[u8]>,
        model: &BpmnModel,
        is_empty: bool,
    ) -> Message {
        let mut message = Message::default();
        let offset = reader.buffer_position();
        let (row, col) = self.get_position(reader, offset as usize);
        message.base_element.xml_row_number = row;
        message.base_element.xml_column_number = col;

        for attr in e.attributes() {
            let Ok(attr) = attr else {
                continue;
            };
            let key_bytes = attr.key.as_ref();
            let key = reader.decoder().decode(key_bytes).unwrap_or_default();
            let local_key = self.get_local_name_bytes(key_bytes, reader);
            let value = attr
                .decode_and_unescape_value(reader.decoder())
                .unwrap_or_default();
            match local_key.as_str() {
                ATTRIBUTE_ID => message.base_element.id = Some(value.into_owned()),
                ATTRIBUTE_NAME => message.name = Some(value.into_owned()),
                "itemRef" => {
                    let raw_value = value.into_owned();
                    if let Some(pos) = raw_value.find(':') {
                        let prefix = &raw_value[..pos];
                        let local_part = &raw_value[pos + 1..];
                        if let Some(namespace) = model.namespaces.get(prefix) {
                            message.item_ref = Some(format!("{}:{}", namespace, local_part));
                        } else {
                            message.item_ref = Some(raw_value);
                        }
                    } else {
                        message.item_ref = Some(raw_value);
                    }
                }
                _ => {
                    let mut ext_attr = ExtensionAttribute::default();
                    ext_attr.name = Some(local_key.clone());
                    ext_attr.value = Some(value.into_owned());
                    if let Some(pos) = key.find(':') {
                        let prefix = &key[..pos];
                        ext_attr.namespace_prefix = Some(prefix.to_string());
                        if let Some(ns) = model.namespaces.get(prefix) {
                            ext_attr.namespace = Some(ns.clone());
                        }
                    }
                    message
                        .base_element
                        .attributes
                        .entry(local_key)
                        .or_default()
                        .push(ext_attr);
                }
            }
        }
        self.ensure_id(&mut message.base_element.id);

        if !is_empty {
            let mut buf = Vec::new();
            loop {
                match reader.read_event_into(&mut buf) {
                    Ok(XmlEvent::End(ref inner_e)) => {
                        let local_name =
                            self.get_local_name_bytes(inner_e.local_name().as_ref(), reader);
                        if local_name == "message" {
                            break;
                        }
                    }
                    Ok(XmlEvent::Eof) => break,
                    _ => {}
                }
                buf.clear();
            }
        }

        message
    }

    fn parse_signal(&self, e: &BytesStart, reader: &mut Reader<&[u8]>, is_empty: bool) -> Signal {
        let mut signal = Signal::default();
        let offset = reader.buffer_position();
        let (row, col) = self.get_position(reader, offset as usize);
        signal.base_element.xml_row_number = row;
        signal.base_element.xml_column_number = col;

        for attr in e.attributes() {
            let Ok(attr) = attr else {
                continue;
            };
            let key = self.get_local_name_bytes(attr.key.as_ref(), reader);
            let value = attr
                .decode_and_unescape_value(reader.decoder())
                .unwrap_or_default();
            match key.as_str() {
                ATTRIBUTE_ID => signal.base_element.id = Some(value.into_owned()),
                ATTRIBUTE_NAME => signal.name = Some(value.into_owned()),
                "scope" => signal.scope = Some(value.into_owned()),
                _ => {}
            }
        }
        self.ensure_id(&mut signal.base_element.id);

        if !is_empty {
            let mut buf = Vec::new();
            loop {
                match reader.read_event_into(&mut buf) {
                    Ok(XmlEvent::End(ref inner_e)) => {
                        let local_name =
                            self.get_local_name_bytes(inner_e.local_name().as_ref(), reader);
                        if local_name == "signal" {
                            break;
                        }
                    }
                    Ok(XmlEvent::Eof) => break,
                    _ => {}
                }
                buf.clear();
            }
        }

        signal
    }

    fn parse_escalation(
        &self,
        e: &BytesStart,
        reader: &mut Reader<&[u8]>,
        is_empty: bool,
    ) -> Escalation {
        let mut escalation = Escalation::default();
        let offset = reader.buffer_position();
        let (row, col) = self.get_position(reader, offset as usize);
        escalation.base_element.xml_row_number = row;
        escalation.base_element.xml_column_number = col;

        for attr in e.attributes() {
            let Ok(attr) = attr else {
                continue;
            };
            let local_key = self.get_local_name_bytes(attr.key.as_ref(), reader);
            let value = attr
                .decode_and_unescape_value(reader.decoder())
                .unwrap_or_default();
            match local_key.as_str() {
                ATTRIBUTE_ID => escalation.base_element.id = Some(value.into_owned()),
                ATTRIBUTE_NAME => escalation.name = Some(value.into_owned()),
                ATTRIBUTE_ESCALATION_CODE => escalation.escalation_code = Some(value.into_owned()),
                _ => {}
            }
        }
        self.ensure_id(&mut escalation.base_element.id);

        if !is_empty {
            let mut buf = Vec::new();
            loop {
                match reader.read_event_into(&mut buf) {
                    Ok(XmlEvent::End(ref inner_e)) => {
                        let local_name =
                            self.get_local_name_bytes(inner_e.local_name().as_ref(), reader);
                        if local_name == ELEMENT_ESCALATION {
                            break;
                        }
                    }
                    Ok(XmlEvent::Eof) => break,
                    _ => {}
                }
                buf.clear();
            }
        }

        escalation
    }

    fn parse_collaboration(
        &self,
        _e: &BytesStart,
        reader: &mut Reader<&[u8]>,
        model: &mut BpmnModel,
    ) {
        let mut buf = Vec::new();
        while let Ok(event) = reader.read_event_into(&mut buf) {
            match event {
                XmlEvent::Start(ref inner_e) | XmlEvent::Empty(ref inner_e) => {
                    let local_name =
                        self.get_local_name_bytes(inner_e.local_name().as_ref(), reader);
                    if local_name == "participant" {
                        let mut pool = Pool::default();
                        let participant_namespaces =
                            self.collect_namespaces_from_start(&model.namespaces, inner_e, reader);
                        let offset = reader.buffer_position();
                        let (row, col) = self.get_position(reader, offset as usize);
                        pool.base_element.xml_row_number = row;
                        pool.base_element.xml_column_number = col;
                        for attr in inner_e.attributes() {
                            let Ok(attr) = attr else {
                                continue;
                            };
                            let local_key = self.get_local_name_bytes(attr.key.as_ref(), reader);
                            let value = attr
                                .decode_and_unescape_value(reader.decoder())
                                .unwrap_or_default();
                            match local_key.as_str() {
                                "id" => pool.base_element.id = Some(value.into_owned()),
                                "name" => pool.name = Some(value.into_owned()),
                                "processRef" => pool.process_ref = Some(value.into_owned()),
                                _ => {}
                            }
                        }
                        self.ensure_id(&mut pool.base_element.id);
                        if !matches!(event, XmlEvent::Empty(_)) {
                            let mut p_buf = Vec::new();
                            loop {
                                match reader.read_event_into(&mut p_buf) {
                                    Ok(XmlEvent::Start(ref pe)) => {
                                        let p_name = self
                                            .get_local_name_bytes(pe.local_name().as_ref(), reader);
                                        if p_name == "extensionElements" {
                                            self.parse_generic_extension_elements_into_base_element(
                                                reader,
                                                &mut pool.base_element,
                                                pe,
                                                &participant_namespaces,
                                            );
                                        }
                                    }
                                    Ok(XmlEvent::End(ref pe)) => {
                                        let p_name = self
                                            .get_local_name_bytes(pe.local_name().as_ref(), reader);
                                        if p_name == "participant" {
                                            break;
                                        }
                                    }
                                    Ok(XmlEvent::Eof) => break,
                                    _ => {}
                                }
                                p_buf.clear();
                            }
                        }
                        model.pools.push(pool);
                    } else if local_name == "messageFlow" {
                        let mut message_flow = MessageFlow::default();
                        let offset = reader.buffer_position();
                        let (row, col) = self.get_position(reader, offset as usize);
                        message_flow.base_element.xml_row_number = row;
                        message_flow.base_element.xml_column_number = col;
                        for attr in inner_e.attributes() {
                            let Ok(attr) = attr else {
                                continue;
                            };
                            let local_key = self.get_local_name_bytes(attr.key.as_ref(), reader);
                            let value = attr
                                .decode_and_unescape_value(reader.decoder())
                                .unwrap_or_default();
                            match local_key.as_str() {
                                "id" => message_flow.base_element.id = Some(value.into_owned()),
                                "name" => message_flow.name = Some(value.into_owned()),
                                "sourceRef" => message_flow.source_ref = Some(value.into_owned()),
                                "targetRef" => message_flow.target_ref = Some(value.into_owned()),
                                "messageRef" => message_flow.message_ref = Some(value.into_owned()),
                                _ => {}
                            }
                        }
                        self.ensure_id(&mut message_flow.base_element.id);
                        if let Some(id) = &message_flow.base_element.id {
                            model.message_flows.insert(id.clone(), message_flow);
                        }
                        if !matches!(event, XmlEvent::Empty(_)) {
                            let mut m_buf = Vec::new();
                            loop {
                                match reader.read_event_into(&mut m_buf) {
                                    Ok(XmlEvent::End(ref me)) => {
                                        let m_name = self
                                            .get_local_name_bytes(me.local_name().as_ref(), reader);
                                        if m_name == "messageFlow" {
                                            break;
                                        }
                                    }
                                    Ok(XmlEvent::Eof) => break,
                                    _ => {}
                                }
                                m_buf.clear();
                            }
                        }
                    }
                }
                XmlEvent::End(ref inner_e) => {
                    let local_name =
                        self.get_local_name_bytes(inner_e.local_name().as_ref(), reader);
                    if local_name == "collaboration" {
                        break;
                    }
                }
                XmlEvent::Eof => break,
                _ => {}
            }
            buf.clear();
        }
    }

    fn parse_lane_set(
        &self,
        _e: &BytesStart,
        reader: &mut Reader<&[u8]>,
        process: &mut Process,
        namespaces: &IndexMap<String, String>,
    ) {
        let mut buf = Vec::new();
        while let Ok(event) = reader.read_event_into(&mut buf) {
            match event {
                XmlEvent::Start(ref inner_e) | XmlEvent::Empty(ref inner_e) => {
                    let local_name =
                        self.get_local_name_bytes(inner_e.local_name().as_ref(), reader);
                    if local_name == "lane" {
                        let mut lane = Lane::default();
                        let lane_namespaces =
                            self.collect_namespaces_from_start(namespaces, inner_e, reader);
                        let offset = reader.buffer_position();
                        let (row, col) = self.get_position(reader, offset as usize);
                        lane.base_element.xml_row_number = row;
                        lane.base_element.xml_column_number = col;
                        for attr in inner_e.attributes() {
                            let Ok(attr) = attr else {
                                continue;
                            };
                            let local_key = self.get_local_name_bytes(attr.key.as_ref(), reader);
                            let value = attr
                                .decode_and_unescape_value(reader.decoder())
                                .unwrap_or_default();
                            match local_key.as_str() {
                                "id" => lane.base_element.id = Some(value.into_owned()),
                                "name" => lane.name = Some(value.into_owned()),
                                _ => {}
                            }
                        }
                        self.ensure_id(&mut lane.base_element.id);

                        if !matches!(event, XmlEvent::Empty(_)) {
                            let mut l_buf = Vec::new();
                            loop {
                                match reader.read_event_into(&mut l_buf) {
                                    Ok(XmlEvent::Start(ref le)) => {
                                        let l_name = self
                                            .get_local_name_bytes(le.local_name().as_ref(), reader);
                                        if l_name == "flowNodeRef" {
                                            lane.flow_references
                                                .push(self.read_element_text(reader, le.name()));
                                        } else if l_name == "extensionElements" {
                                            self.parse_generic_extension_elements_into_base_element(
                                                reader,
                                                &mut lane.base_element,
                                                le,
                                                &lane_namespaces,
                                            );
                                        }
                                    }
                                    Ok(XmlEvent::End(ref le)) => {
                                        let l_name = self
                                            .get_local_name_bytes(le.local_name().as_ref(), reader);
                                        if l_name == "lane" {
                                            break;
                                        }
                                    }
                                    Ok(XmlEvent::Eof) => break,
                                    _ => {}
                                }
                                l_buf.clear();
                            }
                        }
                        process.lanes.push(lane);
                    }
                }
                XmlEvent::End(ref inner_e) => {
                    let local_name =
                        self.get_local_name_bytes(inner_e.local_name().as_ref(), reader);
                    if local_name == "laneSet" {
                        break;
                    }
                }
                XmlEvent::Eof => break,
                _ => {}
            }
            buf.clear();
        }
    }

    fn parse_data_object(
        &self,
        e: &BytesStart,
        reader: &mut Reader<&[u8]>,
        is_empty: bool,
        namespaces: &IndexMap<String, String>,
    ) -> ValuedDataObject {
        let namespaces = self.collect_namespaces_from_start(namespaces, e, reader);
        let mut obj = ValuedDataObject::default();
        let offset = reader.buffer_position();
        let (row, col) = self.get_position(reader, offset as usize);
        obj.base_element.xml_row_number = row;
        obj.base_element.xml_column_number = col;

        for attr in e.attributes() {
            let Ok(attr) = attr else {
                continue;
            };
            let local_key = self.get_local_name_bytes(attr.key.as_ref(), reader);
            let value = attr
                .decode_and_unescape_value(reader.decoder())
                .unwrap_or_default();
            match local_key.as_str() {
                k if k == ATTRIBUTE_ID => obj.base_element.id = Some(value.into_owned()),
                k if k == ATTRIBUTE_NAME => obj.name = Some(value.into_owned()),
                k if k == ATTRIBUTE_DATA_ITEM_REF => {
                    let mut id = ItemDefinition::default();
                    id.structure_ref = Some(value.into_owned());
                    obj.item_subject_ref = id;
                }
                k if k == ATTRIBUTE_DATA_OBJECT_REF => {
                    obj.data_object_ref = Some(value.into_owned());
                }
                _ => {}
            }
        }
        self.ensure_id(&mut obj.base_element.id);

        if let Some(ref sr) = obj.item_subject_ref.structure_ref {
            if let Some(pos) = sr.find(':') {
                obj.data_type = Some(sr[pos + 1..].to_string());
            } else {
                obj.data_type = Some(sr.clone());
            }
        }

        if !is_empty {
            let mut buf = Vec::new();
            loop {
                match reader.read_event_into(&mut buf) {
                    Ok(XmlEvent::Start(ref inner_e)) => {
                        let inner_name_str =
                            self.get_local_name_bytes(inner_e.local_name().as_ref(), reader);
                        if inner_name_str == ELEMENT_DATA_VALUE {
                            let raw = self.read_element_text(reader, inner_e.name());
                            obj.value =
                                Some(convert_data_object_value(&raw, obj.data_type.as_deref()));
                        } else if inner_name_str == "extensionElements" {
                            self.parse_extensions_into_valued_data_object(
                                reader,
                                &mut obj,
                                inner_e,
                                &namespaces,
                            );
                        }
                    }
                    Ok(XmlEvent::End(ref inner_e)) => {
                        let inner_name =
                            self.get_local_name_bytes(inner_e.local_name().as_ref(), reader);
                        if inner_name == ELEMENT_DATA_OBJECT {
                            break;
                        }
                    }
                    Ok(XmlEvent::Eof) => break,
                    _ => {}
                }
                buf.clear();
            }
        }
        obj
    }

    fn parse_extensions_into_valued_data_object(
        &self,
        reader: &mut Reader<&[u8]>,
        obj: &mut ValuedDataObject,
        wrapper: &BytesStart,
        namespaces: &IndexMap<String, String>,
    ) {
        let namespaces = self.collect_namespaces_from_start(namespaces, wrapper, reader);
        let mut buf = Vec::new();
        while let Ok(event) = reader.read_event_into(&mut buf) {
            match event {
                XmlEvent::Start(ref e) | XmlEvent::Empty(ref e) => {
                    let is_empty = matches!(event, XmlEvent::Empty(_));
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    let ext =
                        self.parse_generic_extension_element(e, reader, &namespaces, is_empty);
                    if local_name == "value" {
                        let raw = ext.element_text.unwrap_or_default();
                        obj.value = Some(convert_data_object_value(&raw, obj.data_type.as_deref()));
                        continue;
                    }
                    obj.base_element
                        .extension_elements
                        .entry(local_name)
                        .or_default()
                        .push(ext);
                }
                XmlEvent::End(ref e) => {
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == "extensionElements" {
                        break;
                    }
                }
                XmlEvent::Eof => break,
                _ => {}
            }
            buf.clear();
        }
    }

    fn parse_flow_element(
        &self,
        e: &BytesStart,
        reader: &mut Reader<&[u8]>,
        model: &mut BpmnModel,
        is_empty: bool,
    ) -> Option<FlowElementEnum> {
        let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
        let offset = reader.buffer_position();
        let (row, col) = self.get_position(reader, offset as usize);

        let element = match local_name.as_str() {
            n if n == ELEMENT_SUBPROCESS
                || n == ELEMENT_TRANSACTION
                || n == ELEMENT_ADHOC_SUBPROCESS =>
            {
                let mut sub_process = SubProcess::default();
                sub_process
                    .activity
                    .flow_node
                    .flow_element
                    .base_element
                    .xml_row_number = row;
                sub_process
                    .activity
                    .flow_node
                    .flow_element
                    .base_element
                    .xml_column_number = col;
                self.parse_common_activity_attributes(e, reader, &mut sub_process.activity);

                if n == ELEMENT_ADHOC_SUBPROCESS {
                    let mut adhoc = AdhocSubProcess::default();
                    adhoc.sub_process = sub_process;
                    for attr in e.attributes() {
                        let Ok(attr) = attr else {
                            continue;
                        };
                        let local_key = self.get_local_name_bytes(attr.key.as_ref(), reader);
                        let value = attr
                            .decode_and_unescape_value(reader.decoder())
                            .unwrap_or_default();
                        match local_key.as_str() {
                            k if k == ATTRIBUTE_ORDERING => {
                                adhoc.ordering = Some(value.into_owned())
                            }
                            k if k == ATTRIBUTE_CANCEL_REMAINING_INSTANCES => {
                                adhoc.cancel_remaining_instances = value == ATTRIBUTE_VALUE_TRUE
                            }
                            _ => {}
                        }
                    }
                    self.ensure_id(
                        &mut adhoc
                            .sub_process
                            .activity
                            .flow_node
                            .flow_element
                            .base_element
                            .id,
                    );
                    if !is_empty {
                        self.parse_sub_process_children(
                            reader,
                            &mut adhoc.sub_process,
                            e,
                            Some(&mut adhoc.completion_condition),
                            model,
                            n,
                        );
                    }
                    FlowElementEnum::AdhocSubProcess(adhoc)
                } else if n == ELEMENT_TRANSACTION {
                    self.ensure_id(
                        &mut sub_process.activity.flow_node.flow_element.base_element.id,
                    );
                    if !is_empty {
                        self.parse_sub_process_children(
                            reader,
                            &mut sub_process,
                            e,
                            None,
                            model,
                            n,
                        );
                    }
                    FlowElementEnum::Transaction(Transaction { sub_process })
                } else {
                    // Check for triggeredByEvent attribute to determine if this is an event subprocess
                    let mut triggered_by_event = false;
                    for attr in e.attributes() {
                        let Ok(attr) = attr else {
                            continue;
                        };
                        let local_key = self.get_local_name_bytes(attr.key.as_ref(), reader);
                        let value = attr
                            .decode_and_unescape_value(reader.decoder())
                            .unwrap_or_default();
                        if local_key == "triggeredByEvent" {
                            triggered_by_event = value == ATTRIBUTE_VALUE_TRUE;
                        }
                    }
                    sub_process.triggered_by_event = triggered_by_event;
                    self.ensure_id(
                        &mut sub_process.activity.flow_node.flow_element.base_element.id,
                    );
                    if !is_empty {
                        self.parse_sub_process_children(
                            reader,
                            &mut sub_process,
                            e,
                            None,
                            model,
                            n,
                        );
                    }
                    if triggered_by_event {
                        FlowElementEnum::EventSubProcess(EventSubProcess { sub_process })
                    } else {
                        FlowElementEnum::SubProcess(sub_process)
                    }
                }
            }
            n if n == ELEMENT_EVENT_START => {
                let mut start_event = StartEvent::default();
                start_event
                    .event
                    .flow_node
                    .flow_element
                    .base_element
                    .xml_row_number = row;
                start_event
                    .event
                    .flow_node
                    .flow_element
                    .base_element
                    .xml_column_number = col;
                self.parse_common_flow_node_attributes(e, reader, &mut start_event.event.flow_node);
                for attr in e.attributes() {
                    let Ok(attr) = attr else {
                        continue;
                    };
                    let local_key = self.get_local_name_bytes(attr.key.as_ref(), reader);
                    let value = attr
                        .decode_and_unescape_value(reader.decoder())
                        .unwrap_or_default();
                    match local_key.as_str() {
                        k if k == ATTRIBUTE_EVENT_START_INITIATOR => {
                            start_event.initiator = Some(value.into_owned())
                        }
                        k if k == ATTRIBUTE_FORM_FORMKEY => {
                            start_event.form_key = Some(value.into_owned())
                        }
                        k if k == ATTRIBUTE_SAME_DEPLOYMENT => {
                            start_event.same_deployment = value != ATTRIBUTE_VALUE_FALSE
                        }
                        k if k == "isInterrupting" => {
                            start_event.interrupting = value != ATTRIBUTE_VALUE_FALSE
                        }
                        _ => {}
                    }
                }
                self.ensure_id(&mut start_event.event.flow_node.flow_element.base_element.id);
                if !is_empty {
                    let mut form_properties = Some(&mut start_event.form_properties);
                    self.parse_event_children(
                        reader,
                        &mut start_event.event,
                        &mut None,
                        &mut None,
                        &mut form_properties,
                        e,
                        n,
                        model,
                    );
                }
                FlowElementEnum::StartEvent(start_event)
            }
            n if n == ELEMENT_EVENT_END => {
                let mut end_event = EndEvent::default();
                end_event
                    .event
                    .flow_node
                    .flow_element
                    .base_element
                    .xml_row_number = row;
                end_event
                    .event
                    .flow_node
                    .flow_element
                    .base_element
                    .xml_column_number = col;
                self.parse_common_flow_node_attributes(e, reader, &mut end_event.event.flow_node);
                self.ensure_id(&mut end_event.event.flow_node.flow_element.base_element.id);
                if !is_empty {
                    self.parse_event_children(
                        reader,
                        &mut end_event.event,
                        &mut None,
                        &mut None,
                        &mut None,
                        e,
                        n,
                        model,
                    );
                }
                FlowElementEnum::EndEvent(end_event)
            }
            n if n == ELEMENT_EVENT_CATCH => {
                let mut catch_event = IntermediateCatchEvent::default();
                catch_event
                    .event
                    .flow_node
                    .flow_element
                    .base_element
                    .xml_row_number = row;
                catch_event
                    .event
                    .flow_node
                    .flow_element
                    .base_element
                    .xml_column_number = col;
                self.parse_common_flow_node_attributes(e, reader, &mut catch_event.event.flow_node);
                self.ensure_id(&mut catch_event.event.flow_node.flow_element.base_element.id);
                if !is_empty {
                    self.parse_event_children(
                        reader,
                        &mut catch_event.event,
                        &mut None,
                        &mut None,
                        &mut None,
                        e,
                        n,
                        model,
                    );
                }
                FlowElementEnum::IntermediateCatchEvent(catch_event)
            }
            n if n == ELEMENT_EVENT_THROW => {
                let mut throw_event = IntermediateThrowEvent::default();
                throw_event
                    .event
                    .flow_node
                    .flow_element
                    .base_element
                    .xml_row_number = row;
                throw_event
                    .event
                    .flow_node
                    .flow_element
                    .base_element
                    .xml_column_number = col;
                self.parse_common_flow_node_attributes(e, reader, &mut throw_event.event.flow_node);
                self.ensure_id(&mut throw_event.event.flow_node.flow_element.base_element.id);
                if !is_empty {
                    self.parse_event_children(
                        reader,
                        &mut throw_event.event,
                        &mut None,
                        &mut None,
                        &mut None,
                        e,
                        n,
                        model,
                    );
                }
                FlowElementEnum::IntermediateThrowEvent(throw_event)
            }
            n if n == ELEMENT_EVENT_BOUNDARY => {
                let mut boundary_event = BoundaryEvent::default();
                boundary_event
                    .event
                    .flow_node
                    .flow_element
                    .base_element
                    .xml_row_number = row;
                boundary_event
                    .event
                    .flow_node
                    .flow_element
                    .base_element
                    .xml_column_number = col;
                self.parse_common_flow_node_attributes(
                    e,
                    reader,
                    &mut boundary_event.event.flow_node,
                );
                for attr in e.attributes() {
                    let Ok(attr) = attr else {
                        continue;
                    };
                    let local_key = self.get_local_name_bytes(attr.key.as_ref(), reader);
                    let value = attr
                        .decode_and_unescape_value(reader.decoder())
                        .unwrap_or_default();
                    match local_key.as_str() {
                        k if k == ATTRIBUTE_BOUNDARY_ATTACHEDTOREF => {
                            boundary_event.attached_to_ref_id = Some(value.into_owned())
                        }
                        k if k == ATTRIBUTE_BOUNDARY_CANCELACTIVITY => {
                            boundary_event.cancel_activity = value != ATTRIBUTE_VALUE_FALSE
                        }
                        _ => {}
                    }
                }
                self.ensure_id(&mut boundary_event.event.flow_node.flow_element.base_element.id);
                if !is_empty {
                    let mut in_params = Some(&mut boundary_event.in_parameters);
                    let mut out_params = Some(&mut boundary_event.out_parameters);
                    self.parse_event_children(
                        reader,
                        &mut boundary_event.event,
                        &mut in_params,
                        &mut out_params,
                        &mut None,
                        e,
                        n,
                        model,
                    );
                    // Java parity (`BoundaryEventXMLConverter.java:86-93`):
                    // model-side cancelActivity is forced to false only when
                    // the boundary event has exactly one event definition and
                    // it is an ErrorEventDefinition (size()==1 semantics, not
                    // "any").
                    if let [
                        flowable_bpmn_model::model::EventDefinitionEnum::ErrorEventDefinition(_),
                    ] = boundary_event.event.event_definitions.as_slice()
                    {
                        boundary_event.cancel_activity = false;
                    }
                }
                FlowElementEnum::BoundaryEvent(boundary_event)
            }
            n if n == ELEMENT_TASK => {
                let mut task = Task::default();
                task.activity
                    .flow_node
                    .flow_element
                    .base_element
                    .xml_row_number = row;
                task.activity
                    .flow_node
                    .flow_element
                    .base_element
                    .xml_column_number = col;
                self.parse_common_activity_attributes(e, reader, &mut task.activity);
                self.ensure_id(&mut task.activity.flow_node.flow_element.base_element.id);
                if !is_empty {
                    self.parse_activity_children(reader, &mut task.activity, e, n, model);
                }
                FlowElementEnum::Task(task)
            }
            n if n == ELEMENT_TASK_MANUAL => {
                // Java ManualTaskXMLConverter.java:40-48 — plain task attributes
                // + child elements; the manual-task behavior is a pass-through
                // (Java ManualTaskActivityBehavior extends TaskActivityBehavior).
                let mut manual_task = ManualTask::default();
                manual_task
                    .task
                    .activity
                    .flow_node
                    .flow_element
                    .base_element
                    .xml_row_number = row;
                manual_task
                    .task
                    .activity
                    .flow_node
                    .flow_element
                    .base_element
                    .xml_column_number = col;
                self.parse_common_activity_attributes(e, reader, &mut manual_task.task.activity);
                self.ensure_id(
                    &mut manual_task
                        .task
                        .activity
                        .flow_node
                        .flow_element
                        .base_element
                        .id,
                );
                if !is_empty {
                    self.parse_activity_children(
                        reader,
                        &mut manual_task.task.activity,
                        e,
                        n,
                        model,
                    );
                }
                FlowElementEnum::ManualTask(manual_task)
            }
            n if n == ELEMENT_TASK_RECEIVE => {
                let mut receive_task = ReceiveTask::default();
                receive_task
                    .task
                    .activity
                    .flow_node
                    .flow_element
                    .base_element
                    .xml_row_number = row;
                receive_task
                    .task
                    .activity
                    .flow_node
                    .flow_element
                    .base_element
                    .xml_column_number = col;
                self.parse_common_activity_attributes(e, reader, &mut receive_task.task.activity);
                for attr in e.attributes() {
                    let Ok(attr) = attr else {
                        continue;
                    };
                    let local_key = self.get_local_name_bytes(attr.key.as_ref(), reader);
                    let value = attr
                        .decode_and_unescape_value(reader.decoder())
                        .unwrap_or_default();
                    match local_key.as_str() {
                        "messageRef" => receive_task.message_ref = Some(value.into_owned()),
                        k if k == ATTRIBUTE_TASK_RECEIVE_SKIP_EXPRESSION => {
                            receive_task.skip_expression = Some(value.into_owned())
                        }
                        _ => {}
                    }
                }
                self.ensure_id(
                    &mut receive_task
                        .task
                        .activity
                        .flow_node
                        .flow_element
                        .base_element
                        .id,
                );
                if !is_empty {
                    self.parse_activity_children(
                        reader,
                        &mut receive_task.task.activity,
                        e,
                        n,
                        model,
                    );
                }
                FlowElementEnum::ReceiveTask(receive_task)
            }
            n if n == ELEMENT_TASK_BUSINESS_RULE => {
                let mut business_rule_task = BusinessRuleTask::default();
                business_rule_task
                    .task
                    .activity
                    .flow_node
                    .flow_element
                    .base_element
                    .xml_row_number = row;
                business_rule_task
                    .task
                    .activity
                    .flow_node
                    .flow_element
                    .base_element
                    .xml_column_number = col;
                self.parse_common_activity_attributes(
                    e,
                    reader,
                    &mut business_rule_task.task.activity,
                );
                for attr in e.attributes() {
                    let Ok(attr) = attr else {
                        continue;
                    };
                    let local_key = self.get_local_name_bytes(attr.key.as_ref(), reader);
                    let value = attr
                        .decode_and_unescape_value(reader.decoder())
                        .unwrap_or_default();
                    match local_key.as_str() {
                        k if k == "ruleVariablesInput" => {
                            business_rule_task.input_variables = value
                                .split(',')
                                .map(|item| item.trim().to_string())
                                .filter(|item| !item.is_empty())
                                .collect();
                        }
                        k if k == "decisionRef" => {
                            business_rule_task.decision_ref = Some(value.into_owned())
                        }
                        k if k == "rules" => {
                            business_rule_task.rule_names = value
                                .split(',')
                                .map(|item| item.trim().to_string())
                                .filter(|item| !item.is_empty())
                                .collect();
                        }
                        k if k == "resultVariable" => {
                            business_rule_task.result_variable_name = Some(value.into_owned())
                        }
                        k if k == "exclude" => {
                            business_rule_task.exclude = value == ATTRIBUTE_VALUE_TRUE
                        }
                        k if k == ATTRIBUTE_TASK_SERVICE_CLASS => {
                            business_rule_task.class_name = Some(value.into_owned())
                        }
                        _ => {}
                    }
                }
                self.ensure_id(
                    &mut business_rule_task
                        .task
                        .activity
                        .flow_node
                        .flow_element
                        .base_element
                        .id,
                );
                if !is_empty {
                    self.parse_activity_children(
                        reader,
                        &mut business_rule_task.task.activity,
                        e,
                        n,
                        model,
                    );
                }
                FlowElementEnum::BusinessRuleTask(business_rule_task)
            }
            n if n == ELEMENT_TASK_USER => {
                let mut user_task = UserTask::default();
                user_task
                    .task
                    .activity
                    .flow_node
                    .flow_element
                    .base_element
                    .xml_row_number = row;
                user_task
                    .task
                    .activity
                    .flow_node
                    .flow_element
                    .base_element
                    .xml_column_number = col;
                self.parse_common_activity_attributes(e, reader, &mut user_task.task.activity);
                for attr in e.attributes() {
                    let Ok(attr) = attr else {
                        continue;
                    };
                    let local_key = self.get_local_name_bytes(attr.key.as_ref(), reader);
                    let value = attr
                        .decode_and_unescape_value(reader.decoder())
                        .unwrap_or_default();
                    match local_key.as_str() {
                        k if k == ATTRIBUTE_TASK_USER_ASSIGNEE => {
                            user_task.assignee = Some(value.into_owned())
                        }
                        k if k == ATTRIBUTE_TASK_USER_OWNER => {
                            user_task.owner = Some(value.into_owned())
                        }
                        k if k == ATTRIBUTE_TASK_USER_CANDIDATE_USERS => {
                            user_task.candidate_users =
                                parse_comma_separated_id_list(value.as_ref())
                        }
                        k if k == ATTRIBUTE_TASK_USER_CANDIDATE_GROUPS => {
                            user_task.candidate_groups =
                                parse_comma_separated_id_list(value.as_ref())
                        }
                        k if k == ATTRIBUTE_TASK_USER_PRIORITY => {
                            user_task.priority = Some(value.into_owned())
                        }
                        k if k == ATTRIBUTE_FORM_FORMKEY => {
                            user_task.form_key = Some(value.into_owned())
                        }
                        k if k == ATTRIBUTE_TASK_USER_DUEDATE => {
                            user_task.due_date = Some(value.into_owned())
                        }
                        k if k == ATTRIBUTE_TASK_USER_BUSINESS_CALENDAR_NAME => {
                            user_task.business_calendar_name = Some(value.into_owned())
                        }
                        k if k == ATTRIBUTE_TASK_USER_CATEGORY => {
                            user_task.category = Some(value.into_owned())
                        }
                        k if k == ATTRIBUTE_TASK_SERVICE_EXTENSIONID => {
                            user_task.extension_id = Some(value.into_owned())
                        }
                        k if k == ATTRIBUTE_TASK_USER_SKIP_EXPRESSION => {
                            user_task.skip_expression = Some(value.into_owned())
                        }
                        k if k == ATTRIBUTE_TASK_ID_VARIABLE_NAME => {
                            user_task.task_id_variable_name = Some(value.into_owned())
                        }
                        k if k == ATTRIBUTE_TASK_COMPLETER_VARIABLE_NAME => {
                            user_task.task_completer_variable_name = Some(value.into_owned())
                        }
                        k if k == ATTRIBUTE_SAME_DEPLOYMENT => {
                            user_task.same_deployment = value != ATTRIBUTE_VALUE_FALSE
                        }
                        _ => {}
                    }
                }
                self.ensure_id(
                    &mut user_task
                        .task
                        .activity
                        .flow_node
                        .flow_element
                        .base_element
                        .id,
                );
                user_task.extended = user_task.extension_id.is_some()
                    && !user_task.extension_id.as_ref().unwrap().is_empty();
                if !is_empty {
                    self.parse_user_task_children(reader, &mut user_task, e, n, model);
                }
                FlowElementEnum::UserTask(user_task)
            }
            n if n == ELEMENT_TASK_SERVICE => {
                let mut service_task = ServiceTask::default();
                // Java CaseServiceTask fields (ServiceTaskXMLConverter.java:422-437).
                let mut case_definition_key: Option<String> = None;
                let mut case_instance_name: Option<String> = None;
                let mut case_business_key: Option<String> = None;
                let mut case_inherit_business_key = false;
                let mut case_same_deployment = false;
                let mut case_fallback_to_default_tenant = false;
                let mut case_instance_id_variable_name: Option<String> = None;
                service_task
                    .task
                    .activity
                    .flow_node
                    .flow_element
                    .base_element
                    .xml_row_number = row;
                service_task
                    .task
                    .activity
                    .flow_node
                    .flow_element
                    .base_element
                    .xml_column_number = col;
                self.parse_common_activity_attributes(e, reader, &mut service_task.task.activity);
                for attr in e.attributes() {
                    let Ok(attr) = attr else {
                        continue;
                    };
                    let key = reader.decoder().decode(attr.key.as_ref()).unwrap();
                    let local_key = self.get_local_name_bytes(attr.key.as_ref(), reader);
                    let value = attr
                        .decode_and_unescape_value(reader.decoder())
                        .unwrap_or_default();
                    match local_key.as_str() {
                        k if k == ATTRIBUTE_TASK_SERVICE_EXTENSIONID => {
                            service_task.extension_id = Some(value.into_owned())
                        }
                        k if k == ATTRIBUTE_FORM_FORMKEY => {
                            let raw_value = value.into_owned();
                            service_task.form_key = Some(raw_value.clone());
                            self.push_extension_attribute(
                                &mut service_task
                                    .task
                                    .activity
                                    .flow_node
                                    .flow_element
                                    .base_element,
                                local_key.clone(),
                                &key,
                                raw_value,
                                &model.namespaces,
                            );
                        }
                        k if k == "formFieldValidation" => {
                            let raw_value = value.into_owned();
                            service_task.validate_form_fields = Some(raw_value.clone());
                            self.push_extension_attribute(
                                &mut service_task
                                    .task
                                    .activity
                                    .flow_node
                                    .flow_element
                                    .base_element,
                                local_key,
                                &key,
                                raw_value,
                                &model.namespaces,
                            );
                        }
                        k if k == ATTRIBUTE_TASK_SERVICE_CLASS => {
                            service_task.implementation_type = Some("class".to_string());
                            service_task.implementation = Some(value.into_owned());
                        }
                        k if k == ATTRIBUTE_TASK_SERVICE_EXPRESSION => {
                            service_task.implementation_type = Some("expression".to_string());
                            service_task.implementation = Some(value.into_owned());
                        }
                        k if k == ATTRIBUTE_TASK_SERVICE_DELEGATEEXPRESSION => {
                            service_task.implementation_type =
                                Some("delegateExpression".to_string());
                            service_task.implementation = Some(value.into_owned());
                        }
                        k if k == ATTRIBUTE_TASK_SERVICE_SKIP_EXPRESSION => {
                            service_task.skip_expression = Some(value.into_owned())
                        }
                        k if k == "type" => service_task.task_type = Some(value.into_owned()),
                        // Java ExternalWorkerServiceTask.topic / flowable:topic
                        k if k == "topic" => service_task.topic = Some(value.into_owned()),
                        // Java ExternalWorkerServiceTask.doNotIncludeVariables
                        // (ServiceTaskXMLConverter.convertExternalWorkerTaskXMLProperties)
                        k if k == "doNotIncludeVariables" => {
                            service_task.do_not_include_variables = value == ATTRIBUTE_VALUE_TRUE
                        }
                        k if k == ATTRIBUTE_TASK_SERVICE_USE_LOCAL_SCOPE_FOR_RESULT_VARIABLE => {
                            service_task.use_local_scope_for_result_variable =
                                value == ATTRIBUTE_VALUE_TRUE
                        }
                        k if k == ATTRIBUTE_TASK_SERVICE_RESULT_VARIABLE_NAME => {
                            service_task.result_variable_name = Some(value.into_owned())
                        }
                        k if k == ATTRIBUTE_TASK_SERVICE_STORE_RESULT_AS_TRANSIENT => {
                            service_task.store_result_variable_as_transient =
                                value == ATTRIBUTE_VALUE_TRUE
                        }
                        k if k == "parallelInSameTransaction" => {
                            service_task.parallel_in_same_transaction =
                                Some(value == ATTRIBUTE_VALUE_TRUE)
                        }
                        // Java ServiceTask.isTriggerable / flowable:triggerable — any
                        // class/delegateExpression/send-event service task may wait for
                        // an external trigger instead of leaving after execute (P51 S4).
                        k if k == "triggerable" => {
                            service_task.triggerable = value == ATTRIBUTE_VALUE_TRUE
                        }
                        // Java CaseServiceTask attributes
                        // (ServiceTaskXMLConverter.convertCaseServiceTaskXMLProperties:422-437).
                        k if k == "caseDefinitionKey" => {
                            case_definition_key = Some(value.into_owned())
                        }
                        k if k == "caseInstanceName" => {
                            case_instance_name = Some(value.into_owned())
                        }
                        k if k == "businessKey" => case_business_key = Some(value.into_owned()),
                        k if k == "inheritBusinessKey" => {
                            case_inherit_business_key = value == ATTRIBUTE_VALUE_TRUE
                        }
                        k if k == "sameDeployment" => {
                            case_same_deployment = value == ATTRIBUTE_VALUE_TRUE
                        }
                        k if k == "fallbackToDefaultTenant" => {
                            case_fallback_to_default_tenant = value == ATTRIBUTE_VALUE_TRUE
                        }
                        k if k == "caseInstanceIdVariableName" || k == "idVariableName" => {
                            // Java ATTRIBUTE_ID_VARIABLE_NAME = "idVariableName"
                            case_instance_id_variable_name = Some(value.into_owned())
                        }
                        _ => {}
                    }
                }
                service_task.extended = service_task.extension_id.is_some()
                    && !service_task.extension_id.as_ref().unwrap().is_empty();
                self.ensure_id(
                    &mut service_task
                        .task
                        .activity
                        .flow_node
                        .flow_element
                        .base_element
                        .id,
                );
                if !is_empty {
                    self.parse_service_task_children(reader, &mut service_task, e, n, model);
                }
                // Java ServiceTaskXMLConverter.java:123-124 + :184-185 —
                // type="case" materializes CaseServiceTask (not plain ServiceTask).
                if service_task.task_type.as_deref() == Some("case") {
                    FlowElementEnum::CaseServiceTask(
                        CaseServiceTask {
                            service_task,
                            case_definition_key,
                            case_instance_name,
                            same_deployment: case_same_deployment,
                            business_key: case_business_key,
                            inherit_business_key: case_inherit_business_key,
                            fallback_to_default_tenant: case_fallback_to_default_tenant,
                            case_instance_id_variable_name,
                        }
                        .ensure_case_type(),
                    )
                } else {
                    FlowElementEnum::ServiceTask(service_task)
                }
            }
            n if n == ELEMENT_TASK_SEND => {
                // Java SendTaskXMLConverter.java:42-55 — `type` + webservice
                // implementation / operationRef attributes; children reuse the
                // service-task parser (field extensions, IO parameters, MI).
                let mut send_task = SendTask::default();
                send_task
                    .service_task
                    .task
                    .activity
                    .flow_node
                    .flow_element
                    .base_element
                    .xml_row_number = row;
                send_task
                    .service_task
                    .task
                    .activity
                    .flow_node
                    .flow_element
                    .base_element
                    .xml_column_number = col;
                self.parse_common_activity_attributes(
                    e,
                    reader,
                    &mut send_task.service_task.task.activity,
                );
                for attr in e.attributes() {
                    let Ok(attr) = attr else {
                        continue;
                    };
                    let local_key = self.get_local_name_bytes(attr.key.as_ref(), reader);
                    let value = attr
                        .decode_and_unescape_value(reader.decoder())
                        .unwrap_or_default();
                    match local_key.as_str() {
                        k if k == "type" => {
                            send_task.service_task.task_type = Some(value.into_owned())
                        }
                        // Java SendTaskXMLConverter.java:47-50 — only
                        // `implementation="##WebService"` marks the webservice path.
                        "implementation" if value.as_ref() == "##WebService" => {
                            send_task.service_task.implementation_type =
                                Some("webservice".to_string());
                        }
                        "operationRef" => send_task.operation_ref = Some(value.into_owned()),
                        _ => {}
                    }
                }
                // Java SendTaskParseHandler.java:54-56 — warn (not fail) when the
                // sendTask has no `type` and is not the webservice form. The webservice
                // form itself is rejected at deployment validation (P105 deviation).
                let is_webservice =
                    send_task.service_task.implementation_type.as_deref() == Some("webservice");
                if !is_webservice
                    && send_task
                        .service_task
                        .task_type
                        .as_deref()
                        .is_none_or(|t| t.trim().is_empty())
                {
                    let id = send_task
                        .service_task
                        .task
                        .activity
                        .flow_node
                        .flow_element
                        .base_element
                        .id
                        .clone()
                        .unwrap_or_default();
                    tracing::warn!(
                        "One of the attributes 'type' or 'operation' is mandatory on sendTask {}",
                        id
                    );
                }
                self.ensure_id(
                    &mut send_task
                        .service_task
                        .task
                        .activity
                        .flow_node
                        .flow_element
                        .base_element
                        .id,
                );
                if !is_empty {
                    self.parse_service_task_children(
                        reader,
                        &mut send_task.service_task,
                        e,
                        n,
                        model,
                    );
                }
                FlowElementEnum::SendTask(send_task)
            }
            n if n == ELEMENT_TASK_SCRIPT => {
                let mut script_task = ScriptTask::default();
                script_task
                    .task
                    .activity
                    .flow_node
                    .flow_element
                    .base_element
                    .xml_row_number = row;
                script_task
                    .task
                    .activity
                    .flow_node
                    .flow_element
                    .base_element
                    .xml_column_number = col;
                self.parse_common_activity_attributes(e, reader, &mut script_task.task.activity);
                for attr in e.attributes() {
                    let Ok(attr) = attr else {
                        continue;
                    };
                    let local_key = self.get_local_name_bytes(attr.key.as_ref(), reader);
                    let value = attr
                        .decode_and_unescape_value(reader.decoder())
                        .unwrap_or_default();
                    match local_key.as_str() {
                        "scriptFormat" => script_task.script_format = Some(value.into_owned()),
                        "resultVariable" => script_task.result_variable = Some(value.into_owned()),
                        "autoStoreVariables" => {
                            script_task.auto_store_variables = value == ATTRIBUTE_VALUE_TRUE
                        }
                        "skipExpression" => script_task.skip_expression = Some(value.into_owned()),
                        "doNotIncludeVariables" => {
                            script_task.do_not_include_variables = value == ATTRIBUTE_VALUE_TRUE
                        }
                        _ => {}
                    }
                }
                self.ensure_id(
                    &mut script_task
                        .task
                        .activity
                        .flow_node
                        .flow_element
                        .base_element
                        .id,
                );
                if !is_empty {
                    self.parse_script_task_children(reader, &mut script_task, e, n, model);
                }
                FlowElementEnum::ScriptTask(script_task)
            }
            n if n == ELEMENT_SEQUENCE_FLOW => {
                let mut sequence_flow = SequenceFlow::default();
                sequence_flow.flow_element.base_element.xml_row_number = row;
                sequence_flow.flow_element.base_element.xml_column_number = col;
                for attr in e.attributes() {
                    let Ok(attr) = attr else {
                        continue;
                    };
                    let local_key = self.get_local_name_bytes(attr.key.as_ref(), reader);
                    let value = attr
                        .decode_and_unescape_value(reader.decoder())
                        .unwrap_or_default();
                    match local_key.as_str() {
                        k if k == ATTRIBUTE_ID => {
                            sequence_flow.flow_element.base_element.id = Some(value.into_owned())
                        }
                        k if k == ATTRIBUTE_NAME => {
                            sequence_flow.flow_element.name = Some(value.into_owned())
                        }
                        k if k == ATTRIBUTE_FLOW_SOURCE_REF => {
                            sequence_flow.source_ref = Some(value.into_owned())
                        }
                        k if k == ATTRIBUTE_FLOW_TARGET_REF => {
                            sequence_flow.target_ref = Some(value.into_owned())
                        }
                        // Java SequenceFlowXMLConverter.java:46 — flowable:skipExpression
                        // on sequence flows (engine-side consumption wired in P106).
                        "skipExpression" => {
                            sequence_flow.skip_expression = Some(value.into_owned())
                        }
                        _ => {}
                    }
                }
                self.ensure_id(&mut sequence_flow.flow_element.base_element.id);
                if !is_empty {
                    self.parse_sequence_flow_children(
                        reader,
                        &mut sequence_flow,
                        e,
                        n,
                        &model.namespaces,
                    );
                }
                FlowElementEnum::SequenceFlow(sequence_flow)
            }
            n if n == ELEMENT_GATEWAY_EXCLUSIVE => {
                let mut gateway = ExclusiveGateway::default();
                gateway
                    .gateway
                    .flow_node
                    .flow_element
                    .base_element
                    .xml_row_number = row;
                gateway
                    .gateway
                    .flow_node
                    .flow_element
                    .base_element
                    .xml_column_number = col;
                self.parse_common_flow_node_attributes(e, reader, &mut gateway.gateway.flow_node);
                self.parse_gateway_attributes(e, reader, &mut gateway.gateway);
                self.ensure_id(&mut gateway.gateway.flow_node.flow_element.base_element.id);
                if !is_empty {
                    self.parse_base_element_children(
                        reader,
                        &mut gateway.gateway.flow_node.flow_element,
                        n,
                        model,
                    );
                }
                FlowElementEnum::ExclusiveGateway(gateway)
            }
            n if n == ELEMENT_GATEWAY_PARALLEL => {
                let mut gateway = ParallelGateway::default();
                gateway
                    .gateway
                    .flow_node
                    .flow_element
                    .base_element
                    .xml_row_number = row;
                gateway
                    .gateway
                    .flow_node
                    .flow_element
                    .base_element
                    .xml_column_number = col;
                self.parse_common_flow_node_attributes(e, reader, &mut gateway.gateway.flow_node);
                self.parse_gateway_attributes(e, reader, &mut gateway.gateway);
                self.ensure_id(&mut gateway.gateway.flow_node.flow_element.base_element.id);
                if !is_empty {
                    self.parse_base_element_children(
                        reader,
                        &mut gateway.gateway.flow_node.flow_element,
                        n,
                        model,
                    );
                }
                FlowElementEnum::ParallelGateway(gateway)
            }
            n if n == ELEMENT_GATEWAY_INCLUSIVE => {
                let mut gateway = InclusiveGateway::default();
                gateway
                    .gateway
                    .flow_node
                    .flow_element
                    .base_element
                    .xml_row_number = row;
                gateway
                    .gateway
                    .flow_node
                    .flow_element
                    .base_element
                    .xml_column_number = col;
                self.parse_common_flow_node_attributes(e, reader, &mut gateway.gateway.flow_node);
                self.parse_gateway_attributes(e, reader, &mut gateway.gateway);
                self.ensure_id(&mut gateway.gateway.flow_node.flow_element.base_element.id);
                if !is_empty {
                    self.parse_base_element_children(
                        reader,
                        &mut gateway.gateway.flow_node.flow_element,
                        n,
                        model,
                    );
                }
                FlowElementEnum::InclusiveGateway(gateway)
            }
            n if n == ELEMENT_GATEWAY_EVENT => {
                let mut gateway = EventBasedGateway::default();
                gateway
                    .gateway
                    .flow_node
                    .flow_element
                    .base_element
                    .xml_row_number = row;
                gateway
                    .gateway
                    .flow_node
                    .flow_element
                    .base_element
                    .xml_column_number = col;
                self.parse_common_flow_node_attributes(e, reader, &mut gateway.gateway.flow_node);
                self.parse_gateway_attributes(e, reader, &mut gateway.gateway);
                for attr in e.attributes() {
                    let Ok(attr) = attr else {
                        continue;
                    };
                    let local_key = self.get_local_name_bytes(attr.key.as_ref(), reader);
                    let value = attr
                        .decode_and_unescape_value(reader.decoder())
                        .unwrap_or_default();
                    match local_key.as_str() {
                        "instantiate" => gateway.instantiate = Some(value == ATTRIBUTE_VALUE_TRUE),
                        "eventGatewayType" => gateway.event_gateway_type = Some(value.into_owned()),
                        _ => {}
                    }
                }
                self.ensure_id(&mut gateway.gateway.flow_node.flow_element.base_element.id);
                if !is_empty {
                    self.parse_base_element_children(
                        reader,
                        &mut gateway.gateway.flow_node.flow_element,
                        n,
                        model,
                    );
                }
                FlowElementEnum::EventBasedGateway(gateway)
            }
            n if n == ELEMENT_GATEWAY_COMPLEX => {
                let mut gateway = ComplexGateway::default();
                gateway
                    .gateway
                    .flow_node
                    .flow_element
                    .base_element
                    .xml_row_number = row;
                gateway
                    .gateway
                    .flow_node
                    .flow_element
                    .base_element
                    .xml_column_number = col;
                self.parse_common_flow_node_attributes(e, reader, &mut gateway.gateway.flow_node);
                self.parse_gateway_attributes(e, reader, &mut gateway.gateway);
                self.ensure_id(&mut gateway.gateway.flow_node.flow_element.base_element.id);
                if !is_empty {
                    self.parse_complex_gateway_children(reader, &mut gateway, n, model);
                }
                FlowElementEnum::ComplexGateway(gateway)
            }
            n if n == ELEMENT_CALL_ACTIVITY => {
                let mut call_activity = CallActivity::default();
                call_activity
                    .activity
                    .flow_node
                    .flow_element
                    .base_element
                    .xml_row_number = row;
                call_activity
                    .activity
                    .flow_node
                    .flow_element
                    .base_element
                    .xml_column_number = col;
                self.parse_common_activity_attributes(e, reader, &mut call_activity.activity);
                for attr in e.attributes() {
                    let Ok(attr) = attr else {
                        continue;
                    };
                    let key = reader.decoder().decode(attr.key.as_ref()).unwrap();
                    let local_key = self.get_local_name_bytes(attr.key.as_ref(), reader);
                    let value = attr
                        .decode_and_unescape_value(reader.decoder())
                        .unwrap_or_default();
                    match local_key.as_str() {
                        k if k == ATTRIBUTE_ID => {
                            call_activity
                                .activity
                                .flow_node
                                .flow_element
                                .base_element
                                .id = Some(value.into_owned())
                        }
                        k if k == ATTRIBUTE_NAME => {
                            call_activity.activity.flow_node.flow_element.name =
                                Some(value.into_owned())
                        }
                        k if k == ATTRIBUTE_CALL_ACTIVITY_CALLEDELEMENT => {
                            call_activity.called_element = Some(value.into_owned())
                        }
                        k if k == ATTRIBUTE_CALL_ACTIVITY_CALLEDELEMENTTYPE => {
                            call_activity.called_element_type = Some(value.into_owned())
                        }
                        k if k == "calledElementBinding" => {
                            let value = value.into_owned();
                            call_activity.same_deployment = value == "deployment";
                            let mut ext_attr = ExtensionAttribute::default();
                            ext_attr.name = Some(local_key.clone());
                            ext_attr.value = Some(value);
                            if let Some(pos) = key.find(':') {
                                let prefix = &key[..pos];
                                ext_attr.namespace_prefix = Some(prefix.to_string());
                                if let Some(ns) = model.namespaces.get(prefix) {
                                    ext_attr.namespace = Some(ns.clone());
                                }
                            }
                            call_activity
                                .activity
                                .flow_node
                                .flow_element
                                .base_element
                                .attributes
                                .entry(local_key)
                                .or_default()
                                .push(ext_attr);
                        }
                        k if k == ATTRIBUTE_CALL_ACTIVITY_INHERITVARIABLES => {
                            call_activity.inherit_variables = value == ATTRIBUTE_VALUE_TRUE
                        }
                        k if k == ATTRIBUTE_CALL_ACTIVITY_USE_LOCALSCOPE_FOR_OUTPARAMETERS => {
                            call_activity.use_local_scope_for_out_parameters =
                                value == ATTRIBUTE_VALUE_TRUE
                        }
                        k if k == ATTRIBUTE_CALL_ACTIVITY_COMPLETE_ASYNC => {
                            call_activity.complete_async = value == ATTRIBUTE_VALUE_TRUE
                        }
                        k if k == ATTRIBUTE_FALLBACK_TO_DEFAULT_TENANT => {
                            call_activity.fallback_to_default_tenant =
                                Some(value == ATTRIBUTE_VALUE_TRUE)
                        }
                        // Java: ATTRIBUTE_BUSINESS_KEY = "businessKey" (expression/literal)
                        k if k == ATTRIBUTE_BUSINESS_KEY => {
                            call_activity.business_key = Some(value.into_owned())
                        }
                        // Java: ATTRIBUTE_INHERIT_BUSINESS_KEY = "inheritBusinessKey"
                        k if k == ATTRIBUTE_INHERIT_BUSINESS_KEY => {
                            call_activity.inherit_business_key = value == ATTRIBUTE_VALUE_TRUE
                        }
                        k if k == ATTRIBUTE_CALL_ACTIVITY_PROCESS_INSTANCE_NAME => {
                            call_activity.process_instance_name = Some(value.into_owned())
                        }
                        k if k == ATTRIBUTE_SAME_DEPLOYMENT => {
                            call_activity.same_deployment = value != ATTRIBUTE_VALUE_FALSE
                        }
                        k if k == ATTRIBUTE_CALL_ACTIVITY_PROCESS_INSTANCE_ID_VARIABLE_NAME => {
                            call_activity.process_instance_id_variable_name =
                                Some(value.into_owned())
                        }
                        _ => {
                            let mut ext_attr = ExtensionAttribute::default();
                            ext_attr.name = Some(local_key.clone());
                            ext_attr.value = Some(value.into_owned());
                            if let Some(pos) = key.find(':') {
                                let prefix = &key[..pos];
                                ext_attr.namespace_prefix = Some(prefix.to_string());
                                if let Some(ns) = model.namespaces.get(prefix) {
                                    ext_attr.namespace = Some(ns.clone());
                                }
                            }
                            call_activity
                                .activity
                                .flow_node
                                .flow_element
                                .base_element
                                .attributes
                                .entry(local_key)
                                .or_default()
                                .push(ext_attr);
                        }
                    }
                }
                self.ensure_id(
                    &mut call_activity
                        .activity
                        .flow_node
                        .flow_element
                        .base_element
                        .id,
                );
                if !is_empty {
                    self.parse_call_activity_children(reader, &mut call_activity, e, n, model);
                }
                FlowElementEnum::CallActivity(call_activity)
            }
            _ => return None,
        };
        Some(element)
    }

    #[allow(clippy::collapsible_else_if)]
    fn parse_user_task_children(
        &self,
        reader: &mut Reader<&[u8]>,
        user_task: &mut UserTask,
        wrapper: &BytesStart,
        parent_tag: &str,
        model: &mut BpmnModel,
    ) {
        let namespaces = self.collect_namespaces_from_start(&model.namespaces, wrapper, reader);
        let mut buf = Vec::new();
        loop {
            let event = reader.read_event_into(&mut buf);
            match event {
                Ok(XmlEvent::Start(ref e)) | Ok(XmlEvent::Empty(ref e)) => {
                    let is_empty = matches!(event, Ok(XmlEvent::Empty(_)));
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == ELEMENT_DOCUMENTATION {
                        if !is_empty {
                            user_task.task.activity.flow_node.flow_element.documentation =
                                Some(self.read_element_text(reader, e.name()));
                        }
                    } else if local_name == ELEMENT_FORMPROPERTY {
                        user_task
                            .form_properties
                            .push(self.parse_form_property(e, reader, is_empty));
                    } else if local_name == ELEMENT_EXECUTION_LISTENER {
                        user_task
                            .task
                            .activity
                            .flow_node
                            .flow_element
                            .execution_listeners
                            .push(self.parse_execution_listener(e, reader, false));
                    } else if local_name == ELEMENT_INPUT_ASSOCIATION {
                        user_task
                            .task
                            .activity
                            .data_input_associations
                            .push(self.parse_data_association(e, reader));
                    } else if local_name == ELEMENT_OUTPUT_ASSOCIATION {
                        user_task
                            .task
                            .activity
                            .data_output_associations
                            .push(self.parse_data_association(e, reader));
                    } else if local_name == "extensionElements" {
                        if !is_empty {
                            self.parse_extensions_into_user_task(reader, user_task, e, &namespaces);
                        }
                    } else if local_name == ELEMENT_DATA_OBJECT {
                        let _obj = self.parse_data_object(e, reader, is_empty, &namespaces);
                    } else if local_name == ELEMENT_MULTIINSTANCE {
                        user_task.task.activity.loop_characteristics = Some(
                            self.parse_multi_instance_loop_characteristics(e, reader, is_empty),
                        );
                    } else {
                        if !is_empty {
                            self.skip_element(reader, e.name());
                        }
                    }
                }
                Ok(XmlEvent::End(ref e)) => {
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == parent_tag {
                        break;
                    }
                }
                Ok(XmlEvent::Eof) => break,
                _ => {}
            }
            buf.clear();
        }
    }

    fn parse_call_activity_children(
        &self,
        reader: &mut Reader<&[u8]>,
        call_activity: &mut CallActivity,
        wrapper: &BytesStart,
        parent_tag: &str,
        model: &mut BpmnModel,
    ) {
        let namespaces = self.collect_namespaces_from_start(&model.namespaces, wrapper, reader);
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(XmlEvent::Start(ref e)) => {
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == ELEMENT_DOCUMENTATION {
                        call_activity.activity.flow_node.flow_element.documentation =
                            Some(self.read_element_text(reader, e.name()));
                    } else if local_name == ELEMENT_EXECUTION_LISTENER {
                        call_activity
                            .activity
                            .flow_node
                            .flow_element
                            .execution_listeners
                            .push(self.parse_execution_listener(e, reader, false));
                    } else if local_name == ELEMENT_INPUT_ASSOCIATION {
                        call_activity
                            .activity
                            .data_input_associations
                            .push(self.parse_data_association(e, reader));
                    } else if local_name == ELEMENT_OUTPUT_ASSOCIATION {
                        call_activity
                            .activity
                            .data_output_associations
                            .push(self.parse_data_association(e, reader));
                    } else if local_name == ELEMENT_MULTIINSTANCE {
                        call_activity.activity.loop_characteristics =
                            Some(self.parse_multi_instance_loop_characteristics(e, reader, false));
                    } else if local_name == "extensionElements" {
                        self.parse_extensions_into_call_activity(
                            reader,
                            call_activity,
                            e,
                            &namespaces,
                        );
                    } else if local_name == MAP_EXCEPTION {
                        call_activity
                            .activity
                            .map_exceptions
                            .push(self.parse_map_exception(e, reader));
                    }
                }
                Ok(XmlEvent::End(ref e)) => {
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == parent_tag {
                        break;
                    }
                }
                Ok(XmlEvent::Eof) => break,
                _ => {}
            }
            buf.clear();
        }
    }

    fn parse_extensions_into_process(
        &self,
        reader: &mut Reader<&[u8]>,
        process: &mut Process,
        wrapper: &BytesStart,
        namespaces: &IndexMap<String, String>,
    ) {
        let namespaces = self.collect_namespaces_from_start(namespaces, wrapper, reader);
        let mut buf = Vec::new();
        while let Ok(event) = reader.read_event_into(&mut buf) {
            match event {
                XmlEvent::Start(ref e) | XmlEvent::Empty(ref e) => {
                    let is_empty = matches!(event, XmlEvent::Empty(_));
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == ELEMENT_EXECUTION_LISTENER {
                        process
                            .execution_listeners
                            .push(self.parse_execution_listener(e, reader, is_empty));
                    } else if local_name == ELEMENT_DATA_OBJECT {
                        process.data_objects.push(self.parse_data_object(
                            e,
                            reader,
                            is_empty,
                            &namespaces,
                        ));
                    } else {
                        let ext =
                            self.parse_generic_extension_element(e, reader, &namespaces, is_empty);
                        process
                            .base_element
                            .extension_elements
                            .entry(local_name)
                            .or_default()
                            .push(ext);
                    }
                }
                XmlEvent::End(ref e) => {
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == "extensionElements" {
                        break;
                    }
                }
                XmlEvent::Eof => break,
                _ => {}
            }
            buf.clear();
        }
    }

    fn parse_service_task_children(
        &self,
        reader: &mut Reader<&[u8]>,
        service_task: &mut ServiceTask,
        wrapper: &BytesStart,
        parent_tag: &str,
        model: &mut BpmnModel,
    ) {
        let namespaces = self.collect_namespaces_from_start(&model.namespaces, wrapper, reader);
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(XmlEvent::Start(ref e)) => {
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == ELEMENT_DOCUMENTATION {
                        service_task
                            .task
                            .activity
                            .flow_node
                            .flow_element
                            .documentation = Some(self.read_element_text(reader, e.name()));
                    } else if local_name == ELEMENT_EXECUTION_LISTENER {
                        service_task
                            .task
                            .activity
                            .flow_node
                            .flow_element
                            .execution_listeners
                            .push(self.parse_execution_listener(e, reader, false));
                    } else if local_name == ELEMENT_INPUT_ASSOCIATION {
                        service_task
                            .task
                            .activity
                            .data_input_associations
                            .push(self.parse_data_association(e, reader));
                    } else if local_name == ELEMENT_OUTPUT_ASSOCIATION {
                        service_task
                            .task
                            .activity
                            .data_output_associations
                            .push(self.parse_data_association(e, reader));
                    } else if local_name == "extensionElements" {
                        self.parse_extensions_into_service_task(
                            reader,
                            service_task,
                            e,
                            &namespaces,
                        );
                    } else if local_name == MAP_EXCEPTION {
                        service_task
                            .task
                            .activity
                            .map_exceptions
                            .push(self.parse_map_exception(e, reader));
                    } else if local_name == ELEMENT_MULTIINSTANCE {
                        service_task.task.activity.loop_characteristics =
                            Some(self.parse_multi_instance_loop_characteristics(e, reader, false));
                    }
                }
                Ok(XmlEvent::End(ref e)) => {
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == parent_tag {
                        break;
                    }
                }
                Ok(XmlEvent::Eof) => break,
                _ => {}
            }
            buf.clear();
        }
    }

    fn parse_extensions_into_service_task(
        &self,
        reader: &mut Reader<&[u8]>,
        service_task: &mut ServiceTask,
        wrapper: &BytesStart,
        namespaces: &IndexMap<String, String>,
    ) {
        let namespaces = self.collect_namespaces_from_start(namespaces, wrapper, reader);
        let mut buf = Vec::new();
        while let Ok(event) = reader.read_event_into(&mut buf) {
            match event {
                XmlEvent::Start(ref e) | XmlEvent::Empty(ref e) => {
                    let is_empty = matches!(event, XmlEvent::Empty(_));
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == ELEMENT_EXECUTION_LISTENER {
                        service_task
                            .task
                            .activity
                            .flow_node
                            .flow_element
                            .execution_listeners
                            .push(self.parse_execution_listener(e, reader, is_empty));
                    } else if local_name == ELEMENT_IN_PARAMETERS
                        || local_name == "externalWorkerInParameter"
                    {
                        service_task
                            .in_parameters
                            .push(self.parse_io_parameter(e, reader, is_empty));
                    } else if local_name == "eventInParameter" {
                        service_task
                            .event_in_parameters
                            .push(self.parse_io_parameter(e, reader, is_empty));
                    } else if local_name == "eventOutParameter" {
                        service_task
                            .event_out_parameters
                            .push(self.parse_io_parameter(e, reader, is_empty));
                    } else if local_name == ELEMENT_OUT_PARAMETERS
                        || local_name == "externalWorkerOutParameter"
                    {
                        service_task
                            .out_parameters
                            .push(self.parse_io_parameter(e, reader, is_empty));
                    } else if local_name == "eventType" {
                        if !is_empty {
                            service_task.event_type =
                                Some(self.read_element_text(reader, e.name()).trim().to_string());
                        }
                    } else if local_name == "triggerEventType" {
                        if !is_empty {
                            service_task.trigger_event_type =
                                Some(self.read_element_text(reader, e.name()).trim().to_string());
                        }
                    } else if local_name == "sendSynchronously" {
                        if !is_empty {
                            service_task.send_synchronously =
                                self.read_element_text(reader, e.name()).trim()
                                    == ATTRIBUTE_VALUE_TRUE;
                        }
                    } else if local_name == ELEMENT_FIELD {
                        service_task
                            .task
                            .activity
                            .field_extensions
                            .push(self.parse_field_extension(e, reader, is_empty));
                    } else if local_name == MAP_EXCEPTION {
                        service_task
                            .task
                            .activity
                            .map_exceptions
                            .push(self.parse_map_exception(e, reader));
                    } else if local_name == ELEMENT_FAILED_JOB_RETRY_TIME_CYCLE {
                        if !is_empty {
                            service_task.task.activity.failed_job_retry_time_cycle_value =
                                Some(self.read_element_text(reader, e.name()));
                        }
                    } else {
                        match local_name.as_str() {
                            "mapException"
                            | "externalWorkerInParameter"
                            | "externalWorkerOutParameter" => {
                                if !is_empty {
                                    self.skip_element(reader, e.name());
                                }
                            }
                            _ => {
                                let ext = self.parse_generic_extension_element(
                                    e,
                                    reader,
                                    &namespaces,
                                    is_empty,
                                );
                                if local_name == "httpRequestHandler" {
                                    service_task.http_request_handler =
                                        Some(Self::http_handler_definition(&ext));
                                } else if local_name == "httpResponseHandler" {
                                    service_task.http_response_handler =
                                        Some(Self::http_handler_definition(&ext));
                                }
                                service_task
                                    .task
                                    .activity
                                    .flow_node
                                    .flow_element
                                    .base_element
                                    .extension_elements
                                    .entry(local_name)
                                    .or_default()
                                    .push(ext);
                            }
                        }
                    }
                }
                XmlEvent::End(ref e) => {
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == "extensionElements" {
                        break;
                    }
                }
                XmlEvent::Eof => break,
                _ => {}
            }
            buf.clear();
        }
    }

    fn http_handler_definition(
        extension: &ExtensionElement,
    ) -> flowable_bpmn_model::model::HttpHandlerDefinition {
        let attribute = |name: &str| {
            extension
                .base_element
                .attributes
                .get(name)
                .and_then(|values| values.first())
                .and_then(|attribute| attribute.value.clone())
        };
        let (implementation_type, implementation) = if let Some(value) = attribute("class") {
            (Some("class".to_string()), Some(value))
        } else if let Some(value) = attribute("delegateExpression") {
            (Some("delegateExpression".to_string()), Some(value))
        } else {
            (attribute("type"), None)
        };
        let field_extensions = extension
            .child_elements
            .get("field")
            .into_iter()
            .flatten()
            .map(|field| {
                let field_attribute = |name: &str| {
                    field
                        .base_element
                        .attributes
                        .get(name)
                        .and_then(|values| values.first())
                        .and_then(|attribute| attribute.value.clone())
                };
                FieldExtension {
                    base_element: field.base_element.clone(),
                    field_name: field_attribute("name"),
                    string_value: field_attribute("stringValue").or_else(|| {
                        field
                            .child_elements
                            .get("string")
                            .and_then(|values| values.first())
                            .and_then(|element| element.element_text.clone())
                    }),
                    expression: field_attribute("expression").or_else(|| {
                        field
                            .child_elements
                            .get("expression")
                            .and_then(|values| values.first())
                            .and_then(|element| element.element_text.clone())
                    }),
                }
            })
            .collect();
        let script_info = extension
            .child_elements
            .get("script")
            .and_then(|scripts| scripts.first())
            .map(|script| {
                let script_attribute = |name: &str| {
                    script
                        .base_element
                        .attributes
                        .get(name)
                        .and_then(|values| values.first())
                        .and_then(|attribute| attribute.value.clone())
                };
                flowable_bpmn_model::model::HttpHandlerScriptInfo {
                    language: script_attribute("language"),
                    script: script.element_text.clone(),
                    result_variable: script_attribute("resultVariable"),
                }
            });
        flowable_bpmn_model::model::HttpHandlerDefinition {
            implementation,
            implementation_type,
            field_extensions,
            script_info,
        }
    }

    fn parse_extensions_into_user_task(
        &self,
        reader: &mut Reader<&[u8]>,
        user_task: &mut UserTask,
        wrapper: &BytesStart,
        namespaces: &IndexMap<String, String>,
    ) {
        let namespaces = self.collect_namespaces_from_start(namespaces, wrapper, reader);
        let mut buf = Vec::new();
        while let Ok(event) = reader.read_event_into(&mut buf) {
            match event {
                XmlEvent::Start(ref e) | XmlEvent::Empty(ref e) => {
                    let is_empty = matches!(event, XmlEvent::Empty(_));
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == ELEMENT_EXECUTION_LISTENER {
                        user_task
                            .task
                            .activity
                            .flow_node
                            .flow_element
                            .execution_listeners
                            .push(self.parse_execution_listener(e, reader, is_empty));
                    } else if local_name == ELEMENT_TASK_LISTENER {
                        user_task
                            .task_listeners
                            .push(self.parse_task_listener(e, reader, is_empty));
                    } else if local_name == ELEMENT_FORMPROPERTY {
                        user_task
                            .form_properties
                            .push(self.parse_form_property(e, reader, is_empty));
                    } else if local_name == ELEMENT_FIELD {
                        user_task
                            .task
                            .activity
                            .field_extensions
                            .push(self.parse_field_extension(e, reader, is_empty));
                    } else if local_name == ELEMENT_FAILED_JOB_RETRY_TIME_CYCLE {
                        if !is_empty {
                            user_task.task.activity.failed_job_retry_time_cycle_value =
                                Some(self.read_element_text(reader, e.name()));
                        }
                    } else {
                        match local_name.as_str() {
                            "customResource" | "value" => {
                                if !is_empty {
                                    self.skip_element(reader, e.name());
                                }
                            }
                            _ => {
                                let ext = self.parse_generic_extension_element(
                                    e,
                                    reader,
                                    &namespaces,
                                    is_empty,
                                );
                                user_task
                                    .task
                                    .activity
                                    .flow_node
                                    .flow_element
                                    .base_element
                                    .extension_elements
                                    .entry(local_name)
                                    .or_default()
                                    .push(ext);
                            }
                        }
                    }
                }
                XmlEvent::End(ref e) => {
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == "extensionElements" {
                        break;
                    }
                }
                XmlEvent::Eof => break,
                _ => {}
            }
            buf.clear();
        }
    }

    fn parse_extensions_into_call_activity(
        &self,
        reader: &mut Reader<&[u8]>,
        call_activity: &mut CallActivity,
        wrapper: &BytesStart,
        namespaces: &IndexMap<String, String>,
    ) {
        let namespaces = self.collect_namespaces_from_start(namespaces, wrapper, reader);
        let mut buf = Vec::new();
        while let Ok(event) = reader.read_event_into(&mut buf) {
            match event {
                XmlEvent::Start(ref e) | XmlEvent::Empty(ref e) => {
                    let is_empty = matches!(event, XmlEvent::Empty(_));
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == ELEMENT_EXECUTION_LISTENER {
                        call_activity
                            .activity
                            .flow_node
                            .flow_element
                            .execution_listeners
                            .push(self.parse_execution_listener(e, reader, is_empty));
                    } else if local_name == ELEMENT_IN_PARAMETERS {
                        let mut has_business_key = false;
                        for attr in e.attributes() {
                            let Ok(attr) = attr else {
                                continue;
                            };
                            let local_key = self.get_local_name_bytes(attr.key.as_ref(), reader);
                            let value = attr
                                .decode_and_unescape_value(reader.decoder())
                                .unwrap_or_default();
                            if local_key == "businessKey" {
                                call_activity.business_key = Some(value.into_owned());
                                has_business_key = true;
                            }
                        }
                        if !has_business_key {
                            call_activity
                                .in_parameters
                                .push(self.parse_io_parameter(e, reader, is_empty));
                        }
                    } else if local_name == ELEMENT_OUT_PARAMETERS {
                        call_activity
                            .out_parameters
                            .push(self.parse_io_parameter(e, reader, is_empty));
                    } else if local_name == MAP_EXCEPTION {
                        call_activity
                            .activity
                            .map_exceptions
                            .push(self.parse_map_exception(e, reader));
                    } else if local_name == ELEMENT_FIELD {
                        call_activity
                            .activity
                            .field_extensions
                            .push(self.parse_field_extension(e, reader, is_empty));
                    } else if local_name == ELEMENT_FAILED_JOB_RETRY_TIME_CYCLE && !is_empty {
                        call_activity.activity.failed_job_retry_time_cycle_value =
                            Some(self.read_element_text(reader, e.name()));
                    } else {
                        let ext =
                            self.parse_generic_extension_element(e, reader, &namespaces, is_empty);
                        call_activity
                            .activity
                            .flow_node
                            .flow_element
                            .base_element
                            .extension_elements
                            .entry(local_name)
                            .or_default()
                            .push(ext);
                    }
                }
                XmlEvent::End(ref e) => {
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == "extensionElements" {
                        break;
                    }
                }
                XmlEvent::Eof => break,
                _ => {}
            }
            buf.clear();
        }
    }

    fn collect_namespaces_from_start(
        &self,
        namespaces: &IndexMap<String, String>,
        e: &BytesStart,
        reader: &Reader<&[u8]>,
    ) -> IndexMap<String, String> {
        let mut local_namespaces = namespaces.clone();
        for attr in e.attributes() {
            let Ok(attr) = attr else {
                continue;
            };
            let key = reader.decoder().decode(attr.key.as_ref()).unwrap();
            let value = attr
                .decode_and_unescape_value(reader.decoder())
                .unwrap_or_default();
            if key == "xmlns" {
                local_namespaces.insert(String::new(), value.into_owned());
            } else if let Some(prefix) = key.strip_prefix("xmlns:") {
                local_namespaces.insert(prefix.to_string(), value.into_owned());
            }
        }
        local_namespaces
    }

    fn parse_generic_extension_element(
        &self,
        e: &BytesStart,
        reader: &mut Reader<&[u8]>,
        namespaces: &IndexMap<String, String>,
        is_empty: bool,
    ) -> ExtensionElement {
        let mut local_namespaces = namespaces.clone();
        for attr in e.attributes() {
            let Ok(attr) = attr else {
                continue;
            };
            let a_key = reader.decoder().decode(attr.key.as_ref()).unwrap();
            let a_value = attr
                .decode_and_unescape_value(reader.decoder())
                .unwrap_or_default();
            if a_key == "xmlns" {
                local_namespaces.insert(String::new(), a_value.into_owned());
            } else if let Some(prefix) = a_key.strip_prefix("xmlns:") {
                local_namespaces.insert(prefix.to_string(), a_value.into_owned());
            }
        }

        let mut ext = ExtensionElement::default();
        let offset = reader.buffer_position();
        let (row, col) = self.get_position(reader, offset as usize);
        ext.base_element.xml_row_number = row;
        ext.base_element.xml_column_number = col;

        let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
        ext.name = Some(local_name.clone());

        let name_str = reader
            .decoder()
            .decode(e.name().as_ref())
            .unwrap()
            .into_owned();
        if let Some(pos) = name_str.find(':') {
            ext.namespace_prefix = Some(name_str[..pos].to_string());
            if let Some(namespace) = local_namespaces.get(&name_str[..pos]) {
                ext.namespace = Some(namespace.clone());
            }
        }

        for attr in e.attributes() {
            let Ok(attr) = attr else {
                continue;
            };
            let a_key_bytes = attr.key.as_ref();
            let a_key = reader.decoder().decode(a_key_bytes).unwrap_or_default();
            let a_local_key = self.get_local_name_bytes(a_key_bytes, reader);
            if a_key == "xmlns" || a_key.starts_with("xmlns:") {
                continue;
            }
            let a_value = attr
                .decode_and_unescape_value(reader.decoder())
                .unwrap_or_default();
            let mut ext_attr = ExtensionAttribute::default();
            ext_attr.name = Some(a_local_key.clone());
            ext_attr.value = Some(a_value.into_owned());
            if let Some(pos) = a_key.find(':') {
                ext_attr.namespace_prefix = Some(a_key[..pos].to_string());
                if let Some(namespace) = local_namespaces.get(&a_key[..pos]) {
                    ext_attr.namespace = Some(namespace.clone());
                }
            }
            ext.base_element
                .attributes
                .entry(a_local_key)
                .or_default()
                .push(ext_attr);
        }

        if !is_empty {
            self.parse_generic_extension_children(reader, &mut ext, &local_name, &local_namespaces);
        }

        ext
    }

    fn parse_generic_extension_children(
        &self,
        reader: &mut Reader<&[u8]>,
        ext: &mut ExtensionElement,
        parent_tag: &str,
        namespaces: &IndexMap<String, String>,
    ) {
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(XmlEvent::Start(ref child)) => {
                    let child_local_name =
                        self.get_local_name_bytes(child.local_name().as_ref(), reader);
                    let child_ext =
                        self.parse_generic_extension_element(child, reader, namespaces, false);
                    ext.child_elements
                        .entry(child_local_name)
                        .or_default()
                        .push(child_ext);
                }
                Ok(XmlEvent::Empty(ref child)) => {
                    let child_local_name =
                        self.get_local_name_bytes(child.local_name().as_ref(), reader);
                    let child_ext =
                        self.parse_generic_extension_element(child, reader, namespaces, true);
                    ext.child_elements
                        .entry(child_local_name)
                        .or_default()
                        .push(child_ext);
                }
                Ok(XmlEvent::Text(ref text)) => {
                    let raw = reader
                        .decoder()
                        .decode(text.as_ref())
                        .unwrap()
                        .into_owned()
                        .replace("\r\n", "\n");
                    let trimmed = raw.trim();
                    if !trimmed.is_empty() {
                        if let Some(existing) = ext.element_text.as_mut() {
                            existing.push_str(trimmed);
                        } else {
                            ext.element_text = Some(trimmed.to_string());
                        }
                    }
                }
                Ok(XmlEvent::CData(ref text)) => {
                    let raw = reader
                        .decoder()
                        .decode(text.as_ref())
                        .unwrap()
                        .into_owned()
                        .replace("\r\n", "\n");
                    let trimmed = raw.trim();
                    if !trimmed.is_empty() {
                        if let Some(existing) = ext.element_text.as_mut() {
                            existing.push_str(trimmed);
                        } else {
                            ext.element_text = Some(trimmed.to_string());
                        }
                    }
                }
                Ok(XmlEvent::End(ref end)) => {
                    let end_local_name =
                        self.get_local_name_bytes(end.local_name().as_ref(), reader);
                    if end_local_name == parent_tag {
                        break;
                    }
                }
                Ok(XmlEvent::Eof) => break,
                _ => {}
            }
            buf.clear();
        }
    }

    fn parse_generic_extension_elements_into_base_element(
        &self,
        reader: &mut Reader<&[u8]>,
        base_element: &mut BaseElement,
        wrapper: &BytesStart,
        namespaces: &IndexMap<String, String>,
    ) {
        let namespaces = self.collect_namespaces_from_start(namespaces, wrapper, reader);
        let mut buf = Vec::new();
        while let Ok(event) = reader.read_event_into(&mut buf) {
            match event {
                XmlEvent::Start(ref e) | XmlEvent::Empty(ref e) => {
                    let is_empty = matches!(event, XmlEvent::Empty(_));
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    let ext =
                        self.parse_generic_extension_element(e, reader, &namespaces, is_empty);
                    base_element
                        .extension_elements
                        .entry(local_name)
                        .or_default()
                        .push(ext);
                }
                XmlEvent::End(ref e) => {
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == "extensionElements" {
                        break;
                    }
                }
                XmlEvent::Eof => break,
                _ => {}
            }
            buf.clear();
        }
    }

    fn parse_map_exception(&self, e: &BytesStart, reader: &mut Reader<&[u8]>) -> MapExceptionEntry {
        let mut me = MapExceptionEntry::default();
        for attr in e.attributes() {
            let Ok(attr) = attr else {
                continue;
            };
            let local_key = self.get_local_name_bytes(attr.key.as_ref(), reader);
            let value = attr
                .decode_and_unescape_value(reader.decoder())
                .unwrap_or_default();
            match local_key.as_str() {
                k if k == ATTRIBUTE_ERROR_CODE => me.error_code = Some(value.into_owned()),
                k if k == MAP_EXCEPTION_ANDCHILDREN
                    || k == MAP_EXCEPTION_INCLUDECHILDEXCEPTIONS =>
                {
                    me.and_children = value == ATTRIBUTE_VALUE_TRUE
                }
                k if k == MAP_EXCEPTION_ROOTCAUSE => me.root_cause = Some(value.into_owned()),
                _ => {}
            }
        }
        me.class_name = Some(self.read_element_text(reader, e.name()));
        me
    }

    fn parse_io_parameter(
        &self,
        e: &BytesStart,
        reader: &mut Reader<&[u8]>,
        is_empty: bool,
    ) -> IOParameter {
        let mut io = IOParameter::default();
        let offset = reader.buffer_position();
        let (row, col) = self.get_position(reader, offset as usize);
        io.base_element.xml_row_number = row;
        io.base_element.xml_column_number = col;

        for attr in e.attributes() {
            let Ok(attr) = attr else {
                continue;
            };
            let local_key = self.get_local_name_bytes(attr.key.as_ref(), reader);
            let value = attr
                .decode_and_unescape_value(reader.decoder())
                .unwrap_or_default();
            match local_key.as_str() {
                k if k == ATTRIBUTE_IOPARAMETER_SOURCE => io.source = Some(value.into_owned()),
                k if k == ATTRIBUTE_IOPARAMETER_SOURCE_EXPRESSION => {
                    io.source_expression = Some(value.into_owned())
                }
                k if k == ATTRIBUTE_IOPARAMETER_TARGET => io.target = Some(value.into_owned()),
                k if k == ATTRIBUTE_IOPARAMETER_TARGET_EXPRESSION => {
                    io.target_expression = Some(value.into_owned())
                }
                k if k == ATTRIBUTE_IOPARAMETER_TRANSIENT => {
                    io.transient = value == ATTRIBUTE_VALUE_TRUE
                }
                _ => {}
            }
        }
        self.ensure_id(&mut io.base_element.id);

        if !is_empty {
            let mut buf = Vec::new();
            loop {
                match reader.read_event_into(&mut buf) {
                    Ok(XmlEvent::End(ref inner_e)) => {
                        let local_name =
                            self.get_local_name_bytes(inner_e.local_name().as_ref(), reader);
                        if local_name == ELEMENT_IN_PARAMETERS
                            || local_name == ELEMENT_OUT_PARAMETERS
                            || local_name == "eventInParameter"
                            || local_name == "eventOutParameter"
                        {
                            break;
                        }
                    }
                    Ok(XmlEvent::Eof) => break,
                    _ => {}
                }
                buf.clear();
            }
        }

        io
    }

    fn parse_field_extension(
        &self,
        e: &BytesStart,
        reader: &mut Reader<&[u8]>,
        is_empty: bool,
    ) -> FieldExtension {
        let mut field = FieldExtension::default();
        let offset = reader.buffer_position();
        let (row, col) = self.get_position(reader, offset as usize);
        field.base_element.xml_row_number = row;
        field.base_element.xml_column_number = col;

        for attr in e.attributes() {
            let Ok(attr) = attr else {
                continue;
            };
            let local_key = self.get_local_name_bytes(attr.key.as_ref(), reader);
            let value = attr
                .decode_and_unescape_value(reader.decoder())
                .unwrap_or_default();
            match local_key.as_str() {
                k if k == ATTRIBUTE_FIELD_NAME => field.field_name = Some(value.into_owned()),
                k if k == ATTRIBUTE_FIELD_STRING => field.string_value = Some(value.into_owned()),
                k if k == ATTRIBUTE_FIELD_EXPRESSION => field.expression = Some(value.into_owned()),
                _ => {}
            }
        }
        self.ensure_id(&mut field.base_element.id);

        if !is_empty {
            let mut buf = Vec::new();
            loop {
                match reader.read_event_into(&mut buf) {
                    Ok(XmlEvent::Start(ref inner_e)) => {
                        let local_name =
                            self.get_local_name_bytes(inner_e.local_name().as_ref(), reader);
                        // P86b: accept both `<flowable:string>` (ELEMENT_FIELD_STRING)
                        // and historical `<flowable:stringValue>` child form
                        // (ATTRIBUTE_FIELD_STRING local name). Attribute-style
                        // stringValue on the field element is handled above;
                        // without this branch the child form silently drops the
                        // value. Java FieldExtensionParser primarily emits
                        // `<string>`, but older BPMN files use `<stringValue>`.
                        if local_name == ELEMENT_FIELD_STRING
                            || local_name == ATTRIBUTE_FIELD_STRING
                        {
                            field.string_value =
                                Some(self.read_element_text(reader, inner_e.name()));
                        } else if local_name == ATTRIBUTE_FIELD_EXPRESSION {
                            field.expression = Some(self.read_element_text(reader, inner_e.name()));
                        }
                    }
                    Ok(XmlEvent::End(ref inner_e)) => {
                        let local_name =
                            self.get_local_name_bytes(inner_e.local_name().as_ref(), reader);
                        if local_name == ELEMENT_FIELD {
                            break;
                        }
                    }
                    Ok(XmlEvent::Eof) => break,
                    _ => {}
                }
                buf.clear();
            }
        }
        field
    }

    fn parse_script_task_children(
        &self,
        reader: &mut Reader<&[u8]>,
        script_task: &mut ScriptTask,
        wrapper: &BytesStart,
        parent_tag: &str,
        model: &mut BpmnModel,
    ) {
        let namespaces = self.collect_namespaces_from_start(&model.namespaces, wrapper, reader);
        let mut buf = Vec::new();
        loop {
            let event = reader.read_event_into(&mut buf);
            match event {
                Ok(XmlEvent::Start(ref e)) | Ok(XmlEvent::Empty(ref e)) => {
                    let is_empty = matches!(event, Ok(XmlEvent::Empty(_)));
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == ELEMENT_DOCUMENTATION {
                        script_task
                            .task
                            .activity
                            .flow_node
                            .flow_element
                            .documentation = Some(self.read_element_text(reader, e.name()));
                    } else if local_name == ELEMENT_EXECUTION_LISTENER {
                        script_task
                            .task
                            .activity
                            .flow_node
                            .flow_element
                            .execution_listeners
                            .push(self.parse_execution_listener(e, reader, false));
                    } else if local_name == "script" {
                        script_task.script = Some(
                            self.read_element_text(reader, e.name())
                                .replace("\r\n", "\n"),
                        );
                    } else if local_name == ELEMENT_MULTIINSTANCE {
                        script_task.task.activity.loop_characteristics = Some(
                            self.parse_multi_instance_loop_characteristics(e, reader, is_empty),
                        );
                    } else if local_name == ELEMENT_IN_PARAMETERS {
                        script_task
                            .in_parameters
                            .push(self.parse_io_parameter(e, reader, is_empty));
                    } else if local_name == ELEMENT_OUT_PARAMETERS {
                        script_task
                            .out_parameters
                            .push(self.parse_io_parameter(e, reader, is_empty));
                    } else if local_name == "extensionElements" {
                        self.parse_extensions_into_script_task(reader, script_task, e, &namespaces);
                    }
                }
                Ok(XmlEvent::End(ref e)) => {
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == parent_tag {
                        break;
                    }
                }
                Ok(XmlEvent::Eof) => break,
                _ => {}
            }
            buf.clear();
        }
    }

    fn parse_extensions_into_script_task(
        &self,
        reader: &mut Reader<&[u8]>,
        script_task: &mut ScriptTask,
        wrapper: &BytesStart,
        namespaces: &IndexMap<String, String>,
    ) {
        let namespaces = self.collect_namespaces_from_start(namespaces, wrapper, reader);
        let mut buf = Vec::new();
        while let Ok(event) = reader.read_event_into(&mut buf) {
            match event {
                XmlEvent::Start(ref e) | XmlEvent::Empty(ref e) => {
                    let is_empty = matches!(event, XmlEvent::Empty(_));
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == ELEMENT_EXECUTION_LISTENER {
                        script_task
                            .task
                            .activity
                            .flow_node
                            .flow_element
                            .execution_listeners
                            .push(self.parse_execution_listener(e, reader, is_empty));
                    } else if local_name == MAP_EXCEPTION {
                        script_task
                            .task
                            .activity
                            .map_exceptions
                            .push(self.parse_map_exception(e, reader));
                    } else if local_name == ELEMENT_FIELD {
                        script_task
                            .task
                            .activity
                            .field_extensions
                            .push(self.parse_field_extension(e, reader, is_empty));
                    } else if local_name == ELEMENT_FAILED_JOB_RETRY_TIME_CYCLE {
                        if !is_empty {
                            script_task.task.activity.failed_job_retry_time_cycle_value =
                                Some(self.read_element_text(reader, e.name()));
                        }
                    } else if local_name == ELEMENT_IN_PARAMETERS {
                        script_task
                            .in_parameters
                            .push(self.parse_io_parameter(e, reader, is_empty));
                    } else if local_name == ELEMENT_OUT_PARAMETERS {
                        script_task
                            .out_parameters
                            .push(self.parse_io_parameter(e, reader, is_empty));
                    } else {
                        let extension =
                            self.parse_generic_extension_element(e, reader, &namespaces, is_empty);
                        script_task
                            .task
                            .activity
                            .flow_node
                            .flow_element
                            .base_element
                            .extension_elements
                            .entry(local_name)
                            .or_default()
                            .push(extension);
                    }
                }
                XmlEvent::End(ref e) => {
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == "extensionElements" {
                        break;
                    }
                }
                XmlEvent::Eof => break,
                _ => {}
            }
            buf.clear();
        }
    }

    fn parse_multi_instance_loop_characteristics(
        &self,
        e: &BytesStart,
        reader: &mut Reader<&[u8]>,
        is_empty: bool,
    ) -> MultiInstanceLoopCharacteristics {
        let mut milc = MultiInstanceLoopCharacteristics::default();
        let offset = reader.buffer_position();
        let (row, col) = self.get_position(reader, offset as usize);
        milc.base_element.xml_row_number = row;
        milc.base_element.xml_column_number = col;

        for attr in e.attributes() {
            let Ok(attr) = attr else {
                continue;
            };
            let local_key = self.get_local_name_bytes(attr.key.as_ref(), reader);
            let value = attr
                .decode_and_unescape_value(reader.decoder())
                .unwrap_or_default();
            match local_key.as_str() {
                k if k == ATTRIBUTE_MULTIINSTANCE_SEQUENTIAL => {
                    milc.sequential = value == ATTRIBUTE_VALUE_TRUE
                }
                k if k == ATTRIBUTE_MULTIINSTANCE_COLLECTION => {
                    milc.input_data_item = Some(value.into_owned())
                }
                k if k == ATTRIBUTE_MULTIINSTANCE_ELEMENT_VARIABLE => {
                    milc.element_variable = Some(value.into_owned())
                }
                k if k == ATTRIBUTE_MULTIINSTANCE_INDEX_VARIABLE => {
                    milc.element_index_variable = Some(value.into_owned())
                }
                k if k == ATTRIBUTE_MULTIINSTANCE_NO_WAIT_STATES_ASYNC_LEAVE => {
                    milc.no_wait_states_async_leave = value == ATTRIBUTE_VALUE_TRUE
                }
                _ => {}
            }
        }

        if is_empty {
            return milc;
        }

        let mut buf = Vec::new();
        while let Ok(event) = reader.read_event_into(&mut buf) {
            match event {
                XmlEvent::Start(ref inner_e) | XmlEvent::Empty(ref inner_e) => {
                    let is_inner_empty = matches!(event, XmlEvent::Empty(_));
                    let local_name =
                        self.get_local_name_bytes(inner_e.local_name().as_ref(), reader);
                    if local_name == ELEMENT_MULTIINSTANCE_CARDINALITY {
                        if !is_inner_empty {
                            milc.loop_cardinality =
                                Some(self.read_element_text(reader, inner_e.name()));
                        }
                    } else if local_name == ELEMENT_MULTIINSTANCE_DATAITEM {
                        if !is_inner_empty {
                            milc.input_data_item =
                                Some(self.read_element_text(reader, inner_e.name()));
                        }
                    } else if local_name == ELEMENT_MULTIINSTANCE_CONDITION {
                        if !is_inner_empty {
                            milc.completion_condition =
                                Some(self.read_element_text(reader, inner_e.name()));
                        }
                    } else if local_name == "extensionElements" {
                        self.parse_extensions_into_milc(reader, &mut milc);
                    }
                }
                XmlEvent::End(ref inner_e) => {
                    let local_name =
                        self.get_local_name_bytes(inner_e.local_name().as_ref(), reader);
                    if local_name == ELEMENT_MULTIINSTANCE {
                        break;
                    }
                }
                XmlEvent::Eof => break,
                _ => {}
            }
            buf.clear();
        }
        milc
    }

    fn parse_extensions_into_milc(
        &self,
        reader: &mut Reader<&[u8]>,
        milc: &mut MultiInstanceLoopCharacteristics,
    ) {
        let mut buf = Vec::new();
        while let Ok(event) = reader.read_event_into(&mut buf) {
            match event {
                XmlEvent::Start(ref inner_e) | XmlEvent::Empty(ref inner_e) => {
                    let is_empty = matches!(event, XmlEvent::Empty(_));
                    let local_name =
                        self.get_local_name_bytes(inner_e.local_name().as_ref(), reader);
                    if local_name == ATTRIBUTE_MULTIINSTANCE_COLLECTION {
                        // flowable:collection
                        let mut handler = CollectionHandler::default();
                        for attr in inner_e.attributes() {
                            let Ok(attr) = attr else {
                                continue;
                            };
                            let key = self.get_local_name_bytes(attr.key.as_ref(), reader);
                            let value = attr
                                .decode_and_unescape_value(reader.decoder())
                                .unwrap_or_default();
                            if key == ATTRIBUTE_CLASS {
                                handler.implementation_type = Some("class".to_string());
                                handler.implementation = Some(value.into_owned());
                            } else if key == ATTRIBUTE_DELEGATEEXPRESSION {
                                handler.implementation_type =
                                    Some("delegateExpression".to_string());
                                handler.implementation = Some(value.into_owned());
                            }
                        }
                        if !is_empty {
                            let mut str_buf = Vec::new();
                            loop {
                                match reader.read_event_into(&mut str_buf) {
                                    Ok(XmlEvent::Start(ref e)) => {
                                        let s_name = self
                                            .get_local_name_bytes(e.local_name().as_ref(), reader);
                                        if s_name == ELEMENT_FIELD_STRING {
                                            milc.collection_string =
                                                Some(self.read_element_text(reader, e.name()));
                                        }
                                    }
                                    Ok(XmlEvent::End(ref e)) => {
                                        let s_name = self
                                            .get_local_name_bytes(e.local_name().as_ref(), reader);
                                        if s_name == ATTRIBUTE_MULTIINSTANCE_COLLECTION {
                                            break;
                                        }
                                    }
                                    Ok(XmlEvent::Eof) => break,
                                    _ => {}
                                }
                                str_buf.clear();
                            }
                        }
                        if handler.implementation.is_some() {
                            milc.handler = Some(handler);
                        }
                    } else if local_name == ELEMENT_VARIABLE_AGGREGATION {
                        let mut agg = VariableAggregationDefinition::default();
                        for attr in inner_e.attributes() {
                            let Ok(attr) = attr else {
                                continue;
                            };
                            let key = self.get_local_name_bytes(attr.key.as_ref(), reader);
                            let value = attr
                                .decode_and_unescape_value(reader.decoder())
                                .unwrap_or_default();
                            match key.as_str() {
                                "target" => agg.target = Some(value.into_owned()),
                                "targetExpression" => {
                                    agg.target_expression = Some(value.into_owned())
                                }
                                "class" => {
                                    agg.implementation_type = Some("class".to_string());
                                    agg.implementation = Some(value.into_owned());
                                }
                                "delegateExpression" => {
                                    agg.implementation_type =
                                        Some("delegateExpression".to_string());
                                    agg.implementation = Some(value.into_owned());
                                }
                                "createOverviewVariable" => {
                                    agg.create_overview_variable = value == ATTRIBUTE_VALUE_TRUE
                                }
                                "storeAsTransientVariable" => {
                                    agg.store_as_transient_variable = value == ATTRIBUTE_VALUE_TRUE
                                }
                                _ => {}
                            }
                        }
                        if !is_empty {
                            let mut var_buf = Vec::new();
                            loop {
                                let v_event = reader.read_event_into(&mut var_buf).unwrap();
                                match v_event {
                                    XmlEvent::Start(ref ve) | XmlEvent::Empty(ref ve) => {
                                        let v_name = self
                                            .get_local_name_bytes(ve.local_name().as_ref(), reader);
                                        if v_name == ELEMENT_VARIABLE {
                                            let mut var_def =
                                                VariableAggregationDefinitionVariable::default();
                                            for attr in ve.attributes() {
                                                let Ok(attr) = attr else {
                                                    continue;
                                                };
                                                let key = self.get_local_name_bytes(
                                                    attr.key.as_ref(),
                                                    reader,
                                                );
                                                let value = attr
                                                    .decode_and_unescape_value(reader.decoder())
                                                    .unwrap();
                                                match key.as_str() {
                                                    "source" => {
                                                        var_def.source = Some(value.into_owned())
                                                    }
                                                    "sourceExpression" => {
                                                        var_def.source_expression =
                                                            Some(value.into_owned())
                                                    }
                                                    "target" => {
                                                        var_def.target = Some(value.into_owned())
                                                    }
                                                    "targetExpression" => {
                                                        var_def.target_expression =
                                                            Some(value.into_owned())
                                                    }
                                                    _ => {}
                                                }
                                            }
                                            agg.definitions.push(var_def);
                                            if !matches!(v_event, XmlEvent::Empty(_)) {
                                                let _ = self.read_element_text(reader, ve.name());
                                                // drain text
                                            }
                                        }
                                    }
                                    XmlEvent::End(ref ve) => {
                                        let v_name = self
                                            .get_local_name_bytes(ve.local_name().as_ref(), reader);
                                        if v_name == ELEMENT_VARIABLE_AGGREGATION {
                                            break;
                                        }
                                    }
                                    XmlEvent::Eof => break,
                                    _ => {}
                                }
                                var_buf.clear();
                            }
                        }
                        if milc.aggregations.is_none() {
                            milc.aggregations = Some(VariableAggregationDefinitions::default());
                        }
                        milc.aggregations.as_mut().unwrap().aggregations.push(agg);
                    }
                }
                XmlEvent::End(ref inner_e) => {
                    let local_name =
                        self.get_local_name_bytes(inner_e.local_name().as_ref(), reader);
                    if local_name == "extensionElements" {
                        break;
                    }
                }
                XmlEvent::Eof => break,
                _ => {}
            }
            buf.clear();
        }
    }

    fn parse_activity_children(
        &self,
        reader: &mut Reader<&[u8]>,
        activity: &mut Activity,
        wrapper: &BytesStart,
        parent_tag: &str,
        model: &mut BpmnModel,
    ) {
        let namespaces = self.collect_namespaces_from_start(&model.namespaces, wrapper, reader);
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(XmlEvent::Start(ref e)) => {
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == ELEMENT_DOCUMENTATION {
                        activity.flow_node.flow_element.documentation =
                            Some(self.read_element_text(reader, e.name()));
                    } else if local_name == ELEMENT_EXECUTION_LISTENER {
                        activity
                            .flow_node
                            .flow_element
                            .execution_listeners
                            .push(self.parse_execution_listener(e, reader, false));
                    } else if local_name == ELEMENT_INPUT_ASSOCIATION {
                        activity
                            .data_input_associations
                            .push(self.parse_data_association(e, reader));
                    } else if local_name == ELEMENT_OUTPUT_ASSOCIATION {
                        activity
                            .data_output_associations
                            .push(self.parse_data_association(e, reader));
                    } else if local_name == "extensionElements" {
                        self.parse_extensions_into_activity(reader, activity, e, &namespaces);
                    } else if local_name == MAP_EXCEPTION {
                        activity
                            .map_exceptions
                            .push(self.parse_map_exception(e, reader));
                    } else if local_name == ELEMENT_MULTIINSTANCE {
                        // ReceiveTask / ManualTask / generic Activity children
                        // path (userTask/serviceTask/scriptTask have dedicated
                        // parsers that already handle this).
                        activity.loop_characteristics =
                            Some(self.parse_multi_instance_loop_characteristics(e, reader, false));
                    }
                }
                Ok(XmlEvent::Empty(ref e)) => {
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == ELEMENT_MULTIINSTANCE {
                        activity.loop_characteristics =
                            Some(self.parse_multi_instance_loop_characteristics(e, reader, true));
                    }
                }
                Ok(XmlEvent::End(ref e)) => {
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == parent_tag {
                        break;
                    }
                }
                Ok(XmlEvent::Eof) => break,
                _ => {}
            }
            buf.clear();
        }
    }

    fn parse_extensions_into_activity(
        &self,
        reader: &mut Reader<&[u8]>,
        activity: &mut Activity,
        wrapper: &BytesStart,
        namespaces: &IndexMap<String, String>,
    ) {
        let namespaces = self.collect_namespaces_from_start(namespaces, wrapper, reader);
        let mut buf = Vec::new();
        while let Ok(event) = reader.read_event_into(&mut buf) {
            match event {
                XmlEvent::Start(ref e) | XmlEvent::Empty(ref e) => {
                    let is_empty = matches!(event, XmlEvent::Empty(_));
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == ELEMENT_EXECUTION_LISTENER {
                        activity
                            .flow_node
                            .flow_element
                            .execution_listeners
                            .push(self.parse_execution_listener(e, reader, is_empty));
                    } else if local_name == MAP_EXCEPTION {
                        activity
                            .map_exceptions
                            .push(self.parse_map_exception(e, reader));
                    } else if local_name == ELEMENT_FIELD {
                        activity
                            .field_extensions
                            .push(self.parse_field_extension(e, reader, is_empty));
                    } else if local_name == ELEMENT_FAILED_JOB_RETRY_TIME_CYCLE {
                        if !is_empty {
                            activity.failed_job_retry_time_cycle_value =
                                Some(self.read_element_text(reader, e.name()));
                        }
                    } else if local_name == "externalWorkerInParameter"
                        || local_name == "externalWorkerOutParameter"
                    {
                        if !is_empty {
                            let _ = self.read_element_text(reader, e.name());
                        }
                    } else if local_name == "customResource"
                        || local_name == "taskListener"
                        || local_name == "formProperty"
                        || local_name == "value"
                        || local_name == "value"
                    {
                        if !is_empty {
                            let _ = self.read_element_text(reader, e.name());
                        }
                    } else {
                        let ext =
                            self.parse_generic_extension_element(e, reader, &namespaces, is_empty);
                        activity
                            .flow_node
                            .flow_element
                            .base_element
                            .extension_elements
                            .entry(local_name)
                            .or_default()
                            .push(ext);
                    }
                }
                XmlEvent::End(ref e) => {
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == "extensionElements" {
                        break;
                    }
                }
                XmlEvent::Eof => break,
                _ => {}
            }
            buf.clear();
        }
    }

    fn parse_data_association(
        &self,
        e: &BytesStart,
        reader: &mut Reader<&[u8]>,
    ) -> DataAssociation {
        let mut da = DataAssociation::default();
        // Java's DataAssociationParser reads the id off the element; without this
        // every association would get a fresh uuid from `ensure_id` and the one
        // authored in the XML would be lost.
        da.base_element.id = self.read_id_attribute(e, reader);
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(XmlEvent::Start(ref inner_e)) => {
                    let local_name =
                        self.get_local_name_bytes(inner_e.local_name().as_ref(), reader);
                    if local_name == ELEMENT_SOURCE_REF {
                        da.source_ref = Some(self.read_element_text(reader, inner_e.name()));
                    } else if local_name == ELEMENT_TARGET_REF {
                        da.target_ref = Some(self.read_element_text(reader, inner_e.name()));
                    } else if local_name == ELEMENT_TRANSFORMATION {
                        da.transformation = Some(self.read_element_text(reader, inner_e.name()));
                    } else if local_name == ELEMENT_ASSIGNMENT {
                        da.assignments.push(self.parse_assignment(inner_e, reader));
                    }
                }
                Ok(XmlEvent::End(ref inner_e)) => {
                    let local_name =
                        self.get_local_name_bytes(inner_e.local_name().as_ref(), reader);
                    if local_name == ELEMENT_INPUT_ASSOCIATION
                        || local_name == ELEMENT_OUTPUT_ASSOCIATION
                    {
                        break;
                    }
                }
                Ok(XmlEvent::Eof) => break,
                _ => {}
            }
            buf.clear();
        }
        self.ensure_id(&mut da.base_element.id);
        da
    }

    fn parse_assignment(&self, e: &BytesStart, reader: &mut Reader<&[u8]>) -> Assignment {
        let mut assignment = Assignment::default();
        // Java leaves the assignment id unset, we generate one in `ensure_id`
        // below — reading it back keeps that generated id stable across a
        // write/parse round-trip.
        assignment.base_element.id = self.read_id_attribute(e, reader);
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(XmlEvent::Start(ref inner_e)) => {
                    let local_name =
                        self.get_local_name_bytes(inner_e.local_name().as_ref(), reader);
                    if local_name == ELEMENT_FROM {
                        assignment.from = Some(self.read_element_text(reader, inner_e.name()));
                    } else if local_name == ELEMENT_TO {
                        assignment.to = Some(self.read_element_text(reader, inner_e.name()));
                    }
                }
                Ok(XmlEvent::End(ref inner_e)) => {
                    let local_name =
                        self.get_local_name_bytes(inner_e.local_name().as_ref(), reader);
                    if local_name == ELEMENT_ASSIGNMENT {
                        break;
                    }
                }
                Ok(XmlEvent::Eof) => break,
                _ => {}
            }
            buf.clear();
        }
        self.ensure_id(&mut assignment.base_element.id);
        assignment
    }

    fn parse_form_property(
        &self,
        e: &BytesStart,
        reader: &mut Reader<&[u8]>,
        is_empty: bool,
    ) -> FormProperty {
        let mut fp = FormProperty::default();
        let offset = reader.buffer_position();
        let (row, col) = self.get_position(reader, offset as usize);
        fp.base_element.xml_row_number = row;
        fp.base_element.xml_column_number = col;

        for attr in e.attributes() {
            let Ok(attr) = attr else {
                continue;
            };
            let local_key = self.get_local_name_bytes(attr.key.as_ref(), reader);
            let value = attr
                .decode_and_unescape_value(reader.decoder())
                .unwrap_or_default();
            match local_key.as_str() {
                k if k == ATTRIBUTE_ID => fp.base_element.id = Some(value.into_owned()),
                k if k == ATTRIBUTE_FORM_NAME => fp.name = Some(value.into_owned()),
                k if k == ATTRIBUTE_FORM_TYPE => fp.property_type = Some(value.into_owned()),
                k if k == ATTRIBUTE_FORM_EXPRESSION => fp.expression = Some(value.into_owned()),
                k if k == ATTRIBUTE_FORM_VARIABLE => fp.variable = Some(value.into_owned()),
                k if k == ATTRIBUTE_FORM_DEFAULT => {
                    fp.default_expression = Some(value.into_owned())
                }
                k if k == ATTRIBUTE_FORM_DATEPATTERN => fp.date_pattern = Some(value.into_owned()),
                k if k == ATTRIBUTE_FORM_READABLE => fp.readable = value != ATTRIBUTE_VALUE_FALSE,
                k if k == ATTRIBUTE_FORM_WRITABLE || k == ATTRIBUTE_FORM_WRITEABLE_ALIAS => {
                    fp.writeable = value != ATTRIBUTE_VALUE_FALSE
                }
                k if k == ATTRIBUTE_FORM_REQUIRED => fp.required = value == ATTRIBUTE_VALUE_TRUE,
                _ => {}
            }
        }
        self.ensure_id(&mut fp.base_element.id);

        if !is_empty {
            let mut buf = Vec::new();
            while let Ok(event) = reader.read_event_into(&mut buf) {
                match event {
                    XmlEvent::Start(ref inner_e) | XmlEvent::Empty(ref inner_e) => {
                        let local_name =
                            self.get_local_name_bytes(inner_e.local_name().as_ref(), reader);
                        if local_name == ELEMENT_DATA_VALUE {
                            let mut fv = FormValue::default();
                            for attr in inner_e.attributes() {
                                let Ok(attr) = attr else {
                                    continue;
                                };
                                let lk = self.get_local_name_bytes(attr.key.as_ref(), reader);
                                let val = attr
                                    .decode_and_unescape_value(reader.decoder())
                                    .unwrap_or_default();
                                match lk.as_str() {
                                    ATTRIBUTE_ID => fv.base_element.id = Some(val.into_owned()),
                                    ATTRIBUTE_NAME => fv.name = Some(val.into_owned()),
                                    _ => {}
                                }
                            }
                            fp.form_values.push(fv);
                            if matches!(event, XmlEvent::Start(_)) {
                                self.skip_element(reader, inner_e.name());
                            }
                        }
                    }
                    XmlEvent::End(ref inner_e) => {
                        let local_name =
                            self.get_local_name_bytes(inner_e.local_name().as_ref(), reader);
                        if local_name == ELEMENT_FORMPROPERTY {
                            break;
                        }
                    }
                    XmlEvent::Eof => break,
                    _ => {}
                }
                buf.clear();
            }
        }
        fp
    }

    fn parse_execution_listener(
        &self,
        e: &BytesStart,
        reader: &mut Reader<&[u8]>,
        is_empty: bool,
    ) -> FlowableListener {
        let mut listener = FlowableListener::default();
        let offset = reader.buffer_position();
        let (row, col) = self.get_position(reader, offset as usize);
        listener.base_element.xml_row_number = row;
        listener.base_element.xml_column_number = col;

        for attr in e.attributes() {
            let Ok(attr) = attr else {
                continue;
            };
            let local_key = self.get_local_name_bytes(attr.key.as_ref(), reader);
            let value = attr
                .decode_and_unescape_value(reader.decoder())
                .unwrap_or_default();
            match local_key.as_str() {
                k if k == ATTRIBUTE_ID => listener.base_element.id = Some(value.into_owned()),
                k if k == ATTRIBUTE_LISTENER_EVENT => listener.event = Some(value.into_owned()),
                k if k == ATTRIBUTE_LISTENER_CLASS => {
                    listener.implementation_type = Some("class".to_string());
                    listener.implementation = Some(value.into_owned());
                }
                k if k == ATTRIBUTE_LISTENER_EXPRESSION => {
                    listener.implementation_type = Some("expression".to_string());
                    listener.implementation = Some(value.into_owned());
                }
                k if k == ATTRIBUTE_LISTENER_DELEGATEEXPRESSION => {
                    listener.implementation_type = Some("delegateExpression".to_string());
                    listener.implementation = Some(value.into_owned());
                }
                k if k == ATTRIBUTE_LISTENER_ON_TRANSACTION => {
                    listener.on_transaction = Some(value.into_owned())
                }
                _ => {}
            }
        }
        self.ensure_id(&mut listener.base_element.id);

        if !is_empty {
            let mut buf = Vec::new();
            while let Ok(event) = reader.read_event_into(&mut buf) {
                match event {
                    XmlEvent::Start(ref inner_e) | XmlEvent::Empty(ref inner_e) => {
                        let is_empty = matches!(event, XmlEvent::Empty(_));
                        let local_name =
                            self.get_local_name_bytes(inner_e.local_name().as_ref(), reader);
                        if local_name == ELEMENT_FIELD {
                            listener
                                .field_extensions
                                .push(self.parse_field_extension(inner_e, reader, is_empty));
                        }
                    }
                    XmlEvent::End(ref inner_e) => {
                        let local_name =
                            self.get_local_name_bytes(inner_e.local_name().as_ref(), reader);
                        if local_name == ELEMENT_EXECUTION_LISTENER {
                            break;
                        }
                    }
                    XmlEvent::Eof => break,
                    _ => {}
                }
                buf.clear();
            }
        }

        listener
    }

    fn parse_task_listener(
        &self,
        e: &BytesStart,
        reader: &mut Reader<&[u8]>,
        is_empty: bool,
    ) -> FlowableListener {
        let mut listener = FlowableListener::default();
        let offset = reader.buffer_position();
        let (row, col) = self.get_position(reader, offset as usize);
        listener.base_element.xml_row_number = row;
        listener.base_element.xml_column_number = col;

        for attr in e.attributes() {
            let Ok(attr) = attr else {
                continue;
            };
            let local_key = self.get_local_name_bytes(attr.key.as_ref(), reader);
            let value = attr
                .decode_and_unescape_value(reader.decoder())
                .unwrap_or_default();
            match local_key.as_str() {
                k if k == ATTRIBUTE_ID => listener.base_element.id = Some(value.into_owned()),
                k if k == ATTRIBUTE_LISTENER_EVENT => listener.event = Some(value.into_owned()),
                k if k == ATTRIBUTE_LISTENER_CLASS => {
                    listener.implementation_type = Some("class".to_string());
                    listener.implementation = Some(value.into_owned());
                }
                k if k == ATTRIBUTE_LISTENER_EXPRESSION => {
                    listener.implementation_type = Some("expression".to_string());
                    listener.implementation = Some(value.into_owned());
                }
                k if k == ATTRIBUTE_LISTENER_DELEGATEEXPRESSION => {
                    listener.implementation_type = Some("delegateExpression".to_string());
                    listener.implementation = Some(value.into_owned());
                }
                k if k == ATTRIBUTE_LISTENER_ON_TRANSACTION => {
                    listener.on_transaction = Some(value.into_owned())
                }
                _ => {}
            }
        }
        self.ensure_id(&mut listener.base_element.id);

        if !is_empty {
            let mut buf = Vec::new();
            while let Ok(event) = reader.read_event_into(&mut buf) {
                match event {
                    XmlEvent::Start(ref inner_e) | XmlEvent::Empty(ref inner_e) => {
                        let is_empty = matches!(event, XmlEvent::Empty(_));
                        let local_name =
                            self.get_local_name_bytes(inner_e.local_name().as_ref(), reader);
                        if local_name == ELEMENT_FIELD {
                            listener
                                .field_extensions
                                .push(self.parse_field_extension(inner_e, reader, is_empty));
                        }
                    }
                    XmlEvent::End(ref inner_e) => {
                        let local_name =
                            self.get_local_name_bytes(inner_e.local_name().as_ref(), reader);
                        if local_name == ELEMENT_TASK_LISTENER {
                            break;
                        }
                    }
                    XmlEvent::Eof => break,
                    _ => {}
                }
                buf.clear();
            }
        }

        listener
    }

    fn parse_sequence_flow_children(
        &self,
        reader: &mut Reader<&[u8]>,
        sequence_flow: &mut SequenceFlow,
        wrapper: &BytesStart,
        parent_tag: &str,
        namespaces: &IndexMap<String, String>,
    ) {
        let namespaces = self.collect_namespaces_from_start(namespaces, wrapper, reader);
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(XmlEvent::Start(ref e)) => {
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == ELEMENT_DOCUMENTATION {
                        sequence_flow.flow_element.documentation =
                            Some(self.read_element_text(reader, e.name()));
                    } else if local_name == ELEMENT_FLOW_CONDITION {
                        sequence_flow.condition_expression =
                            Some(self.read_element_text_preserve_whitespace(reader, e.name()));
                    } else if local_name == ELEMENT_EXECUTION_LISTENER {
                        sequence_flow
                            .flow_element
                            .execution_listeners
                            .push(self.parse_execution_listener(e, reader, false));
                    } else if local_name == "extensionElements" {
                        self.parse_extensions_into_flow(reader, sequence_flow, e, &namespaces);
                    }
                }
                Ok(XmlEvent::End(ref e)) => {
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == parent_tag {
                        break;
                    }
                }
                Ok(XmlEvent::Eof) => break,
                _ => {}
            }
            buf.clear();
        }
    }

    fn parse_extensions_into_flow(
        &self,
        reader: &mut Reader<&[u8]>,
        sequence_flow: &mut SequenceFlow,
        wrapper: &BytesStart,
        namespaces: &IndexMap<String, String>,
    ) {
        let namespaces = self.collect_namespaces_from_start(namespaces, wrapper, reader);
        let mut buf = Vec::new();
        while let Ok(event) = reader.read_event_into(&mut buf) {
            match event {
                XmlEvent::Start(ref e) | XmlEvent::Empty(ref e) => {
                    let is_empty = matches!(event, XmlEvent::Empty(_));
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == ELEMENT_EXECUTION_LISTENER {
                        sequence_flow
                            .flow_element
                            .execution_listeners
                            .push(self.parse_execution_listener(e, reader, is_empty));
                    } else {
                        let ext =
                            self.parse_generic_extension_element(e, reader, &namespaces, is_empty);
                        sequence_flow
                            .flow_element
                            .base_element
                            .extension_elements
                            .entry(local_name)
                            .or_default()
                            .push(ext);
                    }
                }
                XmlEvent::End(ref e) => {
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == "extensionElements" {
                        break;
                    }
                }
                XmlEvent::Eof => break,
                _ => {}
            }
            buf.clear();
        }
    }

    fn parse_event_children(
        &self,
        reader: &mut Reader<&[u8]>,
        event: &mut Event,
        in_parameters: &mut Option<&mut Vec<IOParameter>>,
        out_parameters: &mut Option<&mut Vec<IOParameter>>,
        form_properties: &mut Option<&mut Vec<FormProperty>>,
        wrapper: &BytesStart,
        parent_tag: &str,
        model: &BpmnModel,
    ) {
        let namespaces = self.collect_namespaces_from_start(&model.namespaces, wrapper, reader);
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(XmlEvent::Start(ref e)) => {
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == ELEMENT_DOCUMENTATION {
                        event.flow_node.flow_element.documentation =
                            Some(self.read_element_text(reader, e.name()));
                    } else if local_name == ELEMENT_EVENT_TIMERDEFINITION {
                        let timer = self.parse_timer_event_definition(e, reader, &model.namespaces);
                        event
                            .event_definitions
                            .push(EventDefinitionEnum::TimerEventDefinition(timer));
                    } else if local_name == ELEMENT_ERROR_EVENT_DEFINITION {
                        let ed = self.parse_error_event_definition(e, reader);
                        event
                            .event_definitions
                            .push(EventDefinitionEnum::ErrorEventDefinition(ed));
                    } else if local_name == ELEMENT_EXECUTION_LISTENER {
                        event
                            .flow_node
                            .flow_element
                            .execution_listeners
                            .push(self.parse_execution_listener(e, reader, false));
                    } else if local_name == "extensionElements" {
                        self.parse_extensions_into_event(
                            reader,
                            event,
                            in_parameters,
                            out_parameters,
                            form_properties,
                            e,
                            &namespaces,
                        );
                    } else if local_name == ELEMENT_EVENT_MESSAGEDEFINITION {
                        let med = self.parse_message_event_definition(e, reader, false);
                        event
                            .event_definitions
                            .push(EventDefinitionEnum::MessageEventDefinition(med));
                    } else if local_name == ELEMENT_EVENT_SIGNALDEFINITION {
                        let sed = self.parse_signal_event_definition(e, reader, false);
                        event
                            .event_definitions
                            .push(EventDefinitionEnum::SignalEventDefinition(sed));
                    } else if local_name == ELEMENT_EVENT_CANCELDEFINITION {
                        let def = self.parse_cancel_event_definition(e, reader);
                        event
                            .event_definitions
                            .push(EventDefinitionEnum::CancelEventDefinition(def));
                    } else if local_name == ELEMENT_EVENT_COMPENSATEDEFINITION {
                        let def = self.parse_compensate_event_definition(e, reader);
                        event
                            .event_definitions
                            .push(EventDefinitionEnum::CompensateEventDefinition(def));
                    } else if local_name == ELEMENT_EVENT_LINKDEFINITION {
                        let def = self.parse_link_event_definition(e, reader, false);
                        event
                            .event_definitions
                            .push(EventDefinitionEnum::LinkEventDefinition(def));
                    } else if local_name == ELEMENT_EVENT_CONDITIONALDEFINITION {
                        let def = self.parse_conditional_event_definition(e, reader, false);
                        event
                            .event_definitions
                            .push(EventDefinitionEnum::ConditionalEventDefinition(def));
                    } else if local_name == ELEMENT_EVENT_ESCALATIONDEFINITION {
                        let def = self.parse_escalation_event_definition(e, reader, false);
                        event
                            .event_definitions
                            .push(EventDefinitionEnum::EscalationEventDefinition(def));
                    } else if local_name == ELEMENT_EVENT_TERMINATEDEFINITION
                        && parent_tag == ELEMENT_EVENT_END
                    {
                        // Java `TerminateEventDefinitionParser`: only applied
                        // to EndEvent parents.
                        let def = self.parse_terminate_event_definition(e, reader);
                        event
                            .event_definitions
                            .push(EventDefinitionEnum::TerminateEventDefinition(def));
                    }
                }
                Ok(XmlEvent::Empty(ref e)) => {
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == ELEMENT_EVENT_TIMERDEFINITION {
                        let mut timer = TimerEventDefinition::default();
                        self.ensure_id(&mut timer.base_element.id);
                        event
                            .event_definitions
                            .push(EventDefinitionEnum::TimerEventDefinition(timer));
                    } else if local_name == ELEMENT_ERROR_EVENT_DEFINITION {
                        let ed = self.parse_error_event_definition(e, reader);
                        event
                            .event_definitions
                            .push(EventDefinitionEnum::ErrorEventDefinition(ed));
                    } else if local_name == ELEMENT_EVENT_MESSAGEDEFINITION {
                        let med = self.parse_message_event_definition(e, reader, true);
                        event
                            .event_definitions
                            .push(EventDefinitionEnum::MessageEventDefinition(med));
                    } else if local_name == ELEMENT_EVENT_SIGNALDEFINITION {
                        let sed = self.parse_signal_event_definition(e, reader, true);
                        event
                            .event_definitions
                            .push(EventDefinitionEnum::SignalEventDefinition(sed));
                    } else if local_name == ELEMENT_EVENT_CANCELDEFINITION {
                        let def = self.parse_cancel_event_definition(e, reader);
                        event
                            .event_definitions
                            .push(EventDefinitionEnum::CancelEventDefinition(def));
                    } else if local_name == ELEMENT_EVENT_COMPENSATEDEFINITION {
                        let def = self.parse_compensate_event_definition(e, reader);
                        event
                            .event_definitions
                            .push(EventDefinitionEnum::CompensateEventDefinition(def));
                    } else if local_name == ELEMENT_EVENT_LINKDEFINITION {
                        let def = self.parse_link_event_definition(e, reader, true);
                        event
                            .event_definitions
                            .push(EventDefinitionEnum::LinkEventDefinition(def));
                    } else if local_name == ELEMENT_EVENT_CONDITIONALDEFINITION {
                        let def = self.parse_conditional_event_definition(e, reader, true);
                        event
                            .event_definitions
                            .push(EventDefinitionEnum::ConditionalEventDefinition(def));
                    } else if local_name == ELEMENT_EVENT_ESCALATIONDEFINITION {
                        let def = self.parse_escalation_event_definition(e, reader, true);
                        event
                            .event_definitions
                            .push(EventDefinitionEnum::EscalationEventDefinition(def));
                    } else if local_name == ELEMENT_EVENT_TERMINATEDEFINITION
                        && parent_tag == ELEMENT_EVENT_END
                    {
                        // Self-closing `<terminateEventDefinition .../>` is the
                        // common form; attributes are parsed the same way.
                        let def = self.parse_terminate_event_definition(e, reader);
                        event
                            .event_definitions
                            .push(EventDefinitionEnum::TerminateEventDefinition(def));
                    }
                }
                Ok(XmlEvent::End(ref e)) => {
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == parent_tag {
                        break;
                    }
                }
                Ok(XmlEvent::Eof) => break,
                _ => {}
            }
            buf.clear();
        }
    }

    fn parse_extensions_into_event(
        &self,
        reader: &mut Reader<&[u8]>,
        event: &mut Event,
        in_parameters: &mut Option<&mut Vec<IOParameter>>,
        out_parameters: &mut Option<&mut Vec<IOParameter>>,
        form_properties: &mut Option<&mut Vec<FormProperty>>,
        wrapper: &BytesStart,
        namespaces: &IndexMap<String, String>,
    ) {
        let namespaces = self.collect_namespaces_from_start(namespaces, wrapper, reader);
        let mut buf = Vec::new();
        loop {
            let xml_event = reader.read_event_into(&mut buf).unwrap();
            match xml_event {
                XmlEvent::Start(ref e) | XmlEvent::Empty(ref e) => {
                    let is_empty = matches!(xml_event, XmlEvent::Empty(_));
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == ELEMENT_EXECUTION_LISTENER {
                        event
                            .flow_node
                            .flow_element
                            .execution_listeners
                            .push(self.parse_execution_listener(e, reader, is_empty));
                    } else if local_name == ELEMENT_IN_PARAMETERS {
                        let ip = self.parse_io_parameter(e, reader, is_empty);
                        if let Some(params) = in_parameters {
                            params.push(ip);
                        }
                    } else if local_name == ELEMENT_OUT_PARAMETERS {
                        let op = self.parse_io_parameter(e, reader, is_empty);
                        if let Some(params) = out_parameters {
                            params.push(op);
                        }
                    } else if local_name == ELEMENT_EVENT_VARIABLELISTENERDEFINITION {
                        // Java `VariableListenerEventDefinitionParser`: extension on events.
                        let mut def = VariableListenerEventDefinition::default();
                        for attr in e.attributes() {
                            let Ok(attr) = attr else {
                                continue;
                            };
                            let key = self.get_local_name_bytes(attr.key.as_ref(), reader);
                            let value = attr
                                .decode_and_unescape_value(reader.decoder())
                                .unwrap_or_default()
                                .into_owned();
                            match key.as_str() {
                                k if k == ATTRIBUTE_VARIABLE_NAME => {
                                    def.variable_name = Some(value);
                                }
                                k if k == ATTRIBUTE_VARIABLE_CHANGE_TYPE => {
                                    def.variable_change_type = Some(value);
                                }
                                _ => {}
                            }
                        }
                        event
                            .event_definitions
                            .push(EventDefinitionEnum::VariableListenerEventDefinition(def));
                        if !is_empty {
                            let _ = self.read_element_text(reader, e.name());
                        }
                    } else if local_name == ELEMENT_FIELD {
                        // Field extension is not expected on base event, but in case, we add to activity/flownode if supported
                        // To be safe, skip or process. (Base event doesn't have field extensions in model typically)
                    } else if local_name == ELEMENT_FORMPROPERTY {
                        // Java `StartEvent` declares an inline start form here.
                        // Events that cannot hold one pass `None`, and the
                        // property is consumed without being stored.
                        let property = self.parse_form_property(e, reader, is_empty);
                        if let Some(properties) = form_properties.as_mut() {
                            properties.push(property);
                        }
                    } else if local_name == "customResource"
                        || local_name == "taskListener"
                        || local_name == "value"
                    {
                        if !is_empty {
                            let _ = self.read_element_text(reader, e.name());
                        }
                    } else {
                        let ext =
                            self.parse_generic_extension_element(e, reader, &namespaces, is_empty);
                        event
                            .flow_node
                            .flow_element
                            .base_element
                            .extension_elements
                            .entry(local_name)
                            .or_default()
                            .push(ext);
                    }
                }
                XmlEvent::End(ref e) => {
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == "extensionElements" {
                        break;
                    }
                }
                XmlEvent::Eof => break,
                _ => {}
            }
            buf.clear();
        }
    }

    fn parse_error_event_definition(
        &self,
        e: &BytesStart,
        reader: &mut Reader<&[u8]>,
    ) -> ErrorEventDefinition {
        let mut ed = ErrorEventDefinition::default();
        for attr in e.attributes() {
            let Ok(attr) = attr else {
                continue;
            };
            let key = self.get_local_name_bytes(attr.key.as_ref(), reader);
            let value = attr
                .decode_and_unescape_value(reader.decoder())
                .unwrap_or_default();
            match key.as_str() {
                k if k == ATTRIBUTE_ID => ed.base_element.id = Some(value.into_owned()),
                k if k == ATTRIBUTE_ERROR_CODE => ed.error_code = Some(value.into_owned()),
                k if k == ATTRIBUTE_ERROR_REF => ed.error_ref = Some(value.into_owned()),
                k if k == ATTRIBUTE_ERROR_VARIABLE_NAME => {
                    ed.error_variable_name = Some(value.into_owned())
                }
                k if k == ATTRIBUTE_ERROR_VARIABLE_LOCAL_SCOPE => {
                    ed.error_variable_local_scope = value == ATTRIBUTE_VALUE_TRUE
                }
                k if k == ATTRIBUTE_ERROR_VARIABLE_TRANSIENT => {
                    ed.error_variable_transient = value == ATTRIBUTE_VALUE_TRUE
                }
                _ => {}
            }
        }
        self.ensure_id(&mut ed.base_element.id);
        ed
    }

    /// Java `TerminateEventDefinitionParser`: `terminateAll` /
    /// `terminateMultiInstance` are true only for the literal string "true"
    /// (`BpmnXMLUtil.getAttributeValue` accepts both the flowable-namespaced
    /// and un-namespaced attribute; local-name matching covers both here).
    fn parse_terminate_event_definition(
        &self,
        e: &BytesStart,
        reader: &mut Reader<&[u8]>,
    ) -> TerminateEventDefinition {
        let mut def = TerminateEventDefinition::default();
        for attr in e.attributes() {
            let Ok(attr) = attr else {
                continue;
            };
            let key = self.get_local_name_bytes(attr.key.as_ref(), reader);
            let value = attr
                .decode_and_unescape_value(reader.decoder())
                .unwrap_or_default();
            match key.as_str() {
                k if k == ATTRIBUTE_ID => def.base_element.id = Some(value.into_owned()),
                k if k == ATTRIBUTE_TERMINATE_ALL => def.terminate_all = value == "true",
                k if k == ATTRIBUTE_TERMINATE_MULTI_INSTANCE => {
                    def.terminate_multi_instance = value == "true"
                }
                _ => {}
            }
        }
        self.ensure_id(&mut def.base_element.id);
        def
    }

    fn parse_cancel_event_definition(
        &self,
        e: &BytesStart,
        reader: &mut Reader<&[u8]>,
    ) -> CancelEventDefinition {
        let mut def = CancelEventDefinition::default();
        for attr in e.attributes() {
            let Ok(attr) = attr else {
                continue;
            };
            let key = self.get_local_name_bytes(attr.key.as_ref(), reader);
            let value = attr
                .decode_and_unescape_value(reader.decoder())
                .unwrap_or_default();
            match key.as_str() {
                k if k == ATTRIBUTE_ID => def.base_element.id = Some(value.into_owned()),
                _ => {}
            }
        }
        self.ensure_id(&mut def.base_element.id);
        def
    }

    fn parse_compensate_event_definition(
        &self,
        e: &BytesStart,
        reader: &mut Reader<&[u8]>,
    ) -> CompensateEventDefinition {
        let mut def = CompensateEventDefinition::default();
        for attr in e.attributes() {
            let Ok(attr) = attr else {
                continue;
            };
            let key = self.get_local_name_bytes(attr.key.as_ref(), reader);
            let value = attr
                .decode_and_unescape_value(reader.decoder())
                .unwrap_or_default();
            match key.as_str() {
                k if k == ATTRIBUTE_ID => def.base_element.id = Some(value.into_owned()),
                k if k == ATTRIBUTE_ACTIVITY_REF => def.activity_ref = Some(value.into_owned()),
                k if k == ATTRIBUTE_WAIT_FOR_COMPLETION => {
                    def.wait_for_completion = value.into_owned() != "false"
                }
                _ => {}
            }
        }
        self.ensure_id(&mut def.base_element.id);
        def
    }

    fn parse_link_event_definition(
        &self,
        e: &BytesStart,
        reader: &mut Reader<&[u8]>,
        is_empty: bool,
    ) -> LinkEventDefinition {
        let mut def = LinkEventDefinition::default();
        for attr in e.attributes() {
            let Ok(attr) = attr else {
                continue;
            };
            let key = self.get_local_name_bytes(attr.key.as_ref(), reader);
            let value = attr
                .decode_and_unescape_value(reader.decoder())
                .unwrap_or_default();
            match key.as_str() {
                k if k == ATTRIBUTE_ID => def.base_element.id = Some(value.into_owned()),
                k if k == ATTRIBUTE_NAME => def.name = Some(value.into_owned()),
                _ => {}
            }
        }
        self.ensure_id(&mut def.base_element.id);

        if !is_empty {
            let mut buf = Vec::new();
            loop {
                match reader.read_event_into(&mut buf) {
                    Ok(XmlEvent::Start(ref e)) | Ok(XmlEvent::Empty(ref e)) => {
                        let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                        if local_name == "source" {
                            let source = self.read_element_text(reader, e.name());
                            def.source.push(source);
                        } else if local_name == "target" {
                            def.target = Some(self.read_element_text(reader, e.name()));
                        }
                    }
                    Ok(XmlEvent::End(ref e))
                        if self.get_local_name_bytes(e.local_name().as_ref(), reader)
                            == ELEMENT_EVENT_LINKDEFINITION =>
                    {
                        break;
                    }
                    Ok(XmlEvent::Eof) => break,
                    _ => {}
                }
                buf.clear();
            }
        }

        def
    }

    fn parse_conditional_event_definition(
        &self,
        e: &BytesStart,
        reader: &mut Reader<&[u8]>,
        is_empty: bool,
    ) -> ConditionalEventDefinition {
        let mut def = ConditionalEventDefinition::default();
        for attr in e.attributes() {
            let Ok(attr) = attr else {
                continue;
            };
            let key = self.get_local_name_bytes(attr.key.as_ref(), reader);
            let value = attr
                .decode_and_unescape_value(reader.decoder())
                .unwrap_or_default();
            match key.as_str() {
                k if k == ATTRIBUTE_ID => def.base_element.id = Some(value.into_owned()),
                _ => {}
            }
        }
        self.ensure_id(&mut def.base_element.id);

        if !is_empty {
            let mut buf = Vec::new();
            loop {
                match reader.read_event_into(&mut buf) {
                    Ok(XmlEvent::Start(ref e)) | Ok(XmlEvent::Empty(ref e)) => {
                        let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                        if local_name == "condition" {
                            def.condition_expression =
                                Some(self.read_element_text(reader, e.name()));
                        }
                    }
                    Ok(XmlEvent::End(ref e))
                        if self.get_local_name_bytes(e.local_name().as_ref(), reader)
                            == ELEMENT_EVENT_CONDITIONALDEFINITION =>
                    {
                        break;
                    }
                    Ok(XmlEvent::Eof) => break,
                    _ => {}
                }
                buf.clear();
            }
        }

        def
    }

    fn parse_message_event_definition(
        &self,
        e: &BytesStart,
        reader: &mut Reader<&[u8]>,
        is_empty: bool,
    ) -> MessageEventDefinition {
        let mut med = MessageEventDefinition::default();
        for attr in e.attributes() {
            let Ok(attr) = attr else {
                continue;
            };
            let key = self.get_local_name_bytes(attr.key.as_ref(), reader);
            let value = attr
                .decode_and_unescape_value(reader.decoder())
                .unwrap_or_default();
            match key.as_str() {
                k if k == ATTRIBUTE_ID => med.base_element.id = Some(value.into_owned()),
                "messageRef" => med.message_ref = Some(value.into_owned()),
                "messageExpression" => med.message_expression = Some(value.into_owned()),
                _ => {}
            }
        }
        self.ensure_id(&mut med.base_element.id);

        if !is_empty {
            let mut buf = Vec::new();
            while let Ok(event) = reader.read_event_into(&mut buf) {
                match event {
                    XmlEvent::Start(ref e) | XmlEvent::Empty(ref e) => {
                        let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                        if local_name == "extensionElements" {
                            // For now we don't deeply parse message event definition extension elements unless needed
                        }
                    }
                    XmlEvent::End(ref e) => {
                        let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                        if local_name == ELEMENT_EVENT_MESSAGEDEFINITION {
                            break;
                        }
                    }
                    XmlEvent::Eof => break,
                    _ => {}
                }
                buf.clear();
            }
        }
        med
    }

    fn parse_signal_event_definition(
        &self,
        e: &BytesStart,
        reader: &mut Reader<&[u8]>,
        is_empty: bool,
    ) -> SignalEventDefinition {
        let mut sed = SignalEventDefinition::default();
        for attr in e.attributes() {
            let Ok(attr) = attr else {
                continue;
            };
            let key = self.get_local_name_bytes(attr.key.as_ref(), reader);
            let value = attr
                .decode_and_unescape_value(reader.decoder())
                .unwrap_or_default();
            match key.as_str() {
                k if k == ATTRIBUTE_ID => sed.base_element.id = Some(value.into_owned()),
                "signalRef" => sed.signal_ref = Some(value.into_owned()),
                "signalExpression" => sed.signal_expression = Some(value.into_owned()),
                _ => {}
            }
        }
        self.ensure_id(&mut sed.base_element.id);

        if !is_empty {
            let mut buf = Vec::new();
            while let Ok(event) = reader.read_event_into(&mut buf) {
                match event {
                    XmlEvent::Start(ref e) | XmlEvent::Empty(ref e) => {
                        let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                        if local_name == "extensionElements" {
                            // For now we don't deeply parse signal event definition extension elements unless needed
                        }
                    }
                    XmlEvent::End(ref e) => {
                        let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                        if local_name == ELEMENT_EVENT_SIGNALDEFINITION {
                            break;
                        }
                    }
                    XmlEvent::Eof => break,
                    _ => {}
                }
                buf.clear();
            }
        }
        sed
    }

    fn parse_escalation_event_definition(
        &self,
        e: &BytesStart,
        reader: &mut Reader<&[u8]>,
        is_empty: bool,
    ) -> EscalationEventDefinition {
        let mut def = EscalationEventDefinition::default();
        for attr in e.attributes() {
            let Ok(attr) = attr else {
                continue;
            };
            let key = self.get_local_name_bytes(attr.key.as_ref(), reader);
            let value = attr
                .decode_and_unescape_value(reader.decoder())
                .unwrap_or_default();
            match key.as_str() {
                k if k == ATTRIBUTE_ID => def.base_element.id = Some(value.into_owned()),
                k if k == ATTRIBUTE_ESCALATION_REF => def.escalation_ref = Some(value.into_owned()),
                k if k == ATTRIBUTE_ESCALATION_CODE => {
                    def.escalation_code = Some(value.into_owned())
                }
                _ => {}
            }
        }
        self.ensure_id(&mut def.base_element.id);

        if !is_empty {
            let mut buf = Vec::new();
            loop {
                match reader.read_event_into(&mut buf) {
                    Ok(XmlEvent::End(ref e)) => {
                        let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                        if local_name == ELEMENT_EVENT_ESCALATIONDEFINITION {
                            break;
                        }
                    }
                    Ok(XmlEvent::Eof) => break,
                    _ => {}
                }
                buf.clear();
            }
        }

        def
    }

    fn parse_timer_event_definition(
        &self,
        e: &BytesStart,
        reader: &mut Reader<&[u8]>,
        namespaces: &IndexMap<String, String>,
    ) -> TimerEventDefinition {
        let mut timer = TimerEventDefinition::default();

        for attr in e.attributes() {
            let Ok(attr) = attr else {
                continue;
            };
            let local_key = self.get_local_name_bytes(attr.key.as_ref(), reader);
            let value = attr
                .decode_and_unescape_value(reader.decoder())
                .unwrap_or_default();
            if local_key == "businessCalendarName" {
                timer.calendar_name = Some(value.into_owned());
            }
        }

        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(XmlEvent::Start(ref e)) => {
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    match local_name.as_str() {
                        "extensionElements" => {
                            self.parse_generic_extension_elements_into_base_element(
                                reader,
                                &mut timer.base_element,
                                e,
                                namespaces,
                            );
                        }
                        "calendar" => {
                            timer.calendar_name = Some(self.read_element_text(reader, e.name()));
                        }
                        n if n == ELEMENT_TIME_DATE => {
                            timer.time_date = Some(self.read_element_text(reader, e.name()))
                        }
                        n if n == ELEMENT_TIME_DURATION => {
                            timer.time_duration = Some(self.read_element_text(reader, e.name()))
                        }
                        n if n == ELEMENT_TIME_CYCLE => {
                            // Java TimeCycleParser: flowable:endDate / activiti:endDate on
                            // the timeCycle element is the cycle hard stop.
                            for attr in e.attributes() {
                                let Ok(attr) = attr else {
                                    continue;
                                };
                                let attr_key = self.get_local_name_bytes(attr.key.as_ref(), reader);
                                if attr_key == "endDate" {
                                    let value = attr
                                        .decode_and_unescape_value(reader.decoder())
                                        .unwrap_or_default();
                                    timer.end_date = Some(value.into_owned());
                                }
                            }
                            timer.time_cycle = Some(self.read_element_text(reader, e.name()))
                        }
                        _ => {}
                    }
                }
                Ok(XmlEvent::End(ref e)) => {
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == ELEMENT_EVENT_TIMERDEFINITION {
                        break;
                    }
                }
                Ok(XmlEvent::Eof) => break,
                _ => {}
            }
            buf.clear();
        }
        self.ensure_id(&mut timer.base_element.id);
        timer
    }

    fn parse_sub_process_children(
        &self,
        reader: &mut Reader<&[u8]>,
        sub_process: &mut SubProcess,
        wrapper: &BytesStart,
        mut completion_condition: Option<&mut Option<String>>,
        model: &mut BpmnModel,
        parent_tag: &str,
    ) {
        let namespaces = self.collect_namespaces_from_start(&model.namespaces, wrapper, reader);
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(XmlEvent::Start(ref e)) => {
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == ELEMENT_DOCUMENTATION {
                        sub_process.activity.flow_node.flow_element.documentation =
                            Some(self.read_element_text(reader, e.name()));
                    } else if local_name == ELEMENT_COMPLETION_CONDITION {
                        if let Some(inner) = completion_condition.as_mut() {
                            **inner = Some(self.read_element_text(reader, e.name()));
                        }
                    } else if local_name == ELEMENT_EXECUTION_LISTENER {
                        sub_process
                            .activity
                            .flow_node
                            .flow_element
                            .execution_listeners
                            .push(self.parse_execution_listener(e, reader, false));
                    } else if local_name == ELEMENT_DATA_OBJECT {
                        let obj = self.parse_data_object(e, reader, false, &namespaces);
                        sub_process.data_objects.push(obj.clone());
                        sub_process
                            .flow_elements
                            .push(FlowElementEnum::ValuedDataObject(obj));
                    } else if local_name == ELEMENT_MULTIINSTANCE {
                        sub_process.activity.loop_characteristics =
                            Some(self.parse_multi_instance_loop_characteristics(e, reader, false));
                    } else if local_name == "extensionElements" {
                        self.parse_extensions_into_sub_process(reader, sub_process, e, &namespaces);
                    } else if let Some(artifact) =
                        self.parse_artifact(e, reader, false, &namespaces)
                    {
                        sub_process.artifacts.push(artifact);
                    } else if let Some(elem) = self.parse_flow_element(e, reader, model, false) {
                        sub_process.flow_elements.push(elem);
                    }
                }
                Ok(XmlEvent::Empty(ref e)) => {
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == ELEMENT_DATA_OBJECT {
                        let obj = self.parse_data_object(e, reader, true, &namespaces);
                        sub_process.data_objects.push(obj.clone());
                        sub_process
                            .flow_elements
                            .push(FlowElementEnum::ValuedDataObject(obj));
                    } else if local_name == ELEMENT_MULTIINSTANCE {
                        // Self-closing multiInstanceLoopCharacteristics on
                        // SubProcess (common for collection/elementVariable form).
                        // Start branch already handled the non-empty form.
                        sub_process.activity.loop_characteristics =
                            Some(self.parse_multi_instance_loop_characteristics(e, reader, true));
                    } else if let Some(artifact) = self.parse_artifact(e, reader, true, &namespaces)
                    {
                        sub_process.artifacts.push(artifact);
                    } else if let Some(elem) = self.parse_flow_element(e, reader, model, true) {
                        sub_process.flow_elements.push(elem);
                    }
                }
                Ok(XmlEvent::End(ref e)) => {
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == parent_tag {
                        break;
                    }
                }
                Ok(XmlEvent::Eof) => break,
                _ => {}
            }
            buf.clear();
        }
    }

    fn parse_extensions_into_sub_process(
        &self,
        reader: &mut Reader<&[u8]>,
        sub_process: &mut SubProcess,
        wrapper: &BytesStart,
        namespaces: &IndexMap<String, String>,
    ) {
        let namespaces = self.collect_namespaces_from_start(namespaces, wrapper, reader);
        let mut buf = Vec::new();
        while let Ok(event) = reader.read_event_into(&mut buf) {
            match event {
                XmlEvent::Start(ref e) | XmlEvent::Empty(ref e) => {
                    let is_empty = matches!(event, XmlEvent::Empty(_));
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == ELEMENT_EXECUTION_LISTENER {
                        sub_process
                            .activity
                            .flow_node
                            .flow_element
                            .execution_listeners
                            .push(self.parse_execution_listener(e, reader, is_empty));
                    } else if local_name == ELEMENT_DATA_OBJECT {
                        sub_process.data_objects.push(self.parse_data_object(
                            e,
                            reader,
                            is_empty,
                            &namespaces,
                        ));
                    } else if local_name == MAP_EXCEPTION {
                        sub_process
                            .activity
                            .map_exceptions
                            .push(self.parse_map_exception(e, reader));
                    } else if local_name == ELEMENT_FIELD {
                        sub_process
                            .activity
                            .field_extensions
                            .push(self.parse_field_extension(e, reader, is_empty));
                    } else if local_name == ELEMENT_FAILED_JOB_RETRY_TIME_CYCLE {
                        if !is_empty {
                            sub_process.activity.failed_job_retry_time_cycle_value =
                                Some(self.read_element_text(reader, e.name()));
                        }
                    } else if local_name == "customResource"
                        || local_name == "taskListener"
                        || local_name == "formProperty"
                        || local_name == "value"
                    {
                        if !is_empty {
                            let _ = self.read_element_text(reader, e.name());
                        }
                    } else {
                        let ext =
                            self.parse_generic_extension_element(e, reader, &namespaces, is_empty);
                        sub_process
                            .activity
                            .flow_node
                            .flow_element
                            .base_element
                            .extension_elements
                            .entry(local_name)
                            .or_default()
                            .push(ext);
                    }
                }
                XmlEvent::End(ref e) => {
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == "extensionElements" {
                        break;
                    }
                }
                XmlEvent::Eof => break,
                _ => {}
            }
            buf.clear();
        }
    }

    fn parse_base_element_children(
        &self,
        reader: &mut Reader<&[u8]>,
        element: &mut FlowElement,
        parent_tag: &str,
        model: &mut BpmnModel,
    ) {
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(XmlEvent::Start(ref e)) => {
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == ELEMENT_DOCUMENTATION {
                        element.documentation = Some(self.read_element_text(reader, e.name()));
                    } else if local_name == ELEMENT_EXECUTION_LISTENER {
                        element
                            .execution_listeners
                            .push(self.parse_execution_listener(e, reader, false));
                    } else if local_name == "extensionElements" {
                        self.parse_extension_elements_base(reader, element, &model.namespaces);
                    }
                }
                Ok(XmlEvent::End(ref e)) => {
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == parent_tag {
                        break;
                    }
                }
                Ok(XmlEvent::Eof) => break,
                _ => {}
            }
            buf.clear();
        }
    }

    fn parse_complex_gateway_children(
        &self,
        reader: &mut Reader<&[u8]>,
        gateway: &mut ComplexGateway,
        parent_tag: &str,
        model: &mut BpmnModel,
    ) {
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(XmlEvent::Start(ref e)) => {
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == ELEMENT_DOCUMENTATION {
                        gateway.gateway.flow_node.flow_element.documentation =
                            Some(self.read_element_text(reader, e.name()));
                    } else if local_name == "activationCondition" {
                        gateway.activation_condition =
                            Some(self.read_element_text(reader, e.name()));
                    } else if local_name == ELEMENT_EXECUTION_LISTENER {
                        gateway
                            .gateway
                            .flow_node
                            .flow_element
                            .execution_listeners
                            .push(self.parse_execution_listener(e, reader, false));
                    } else if local_name == "extensionElements" {
                        self.parse_extension_elements_base(
                            reader,
                            &mut gateway.gateway.flow_node.flow_element,
                            &model.namespaces,
                        );
                    }
                }
                Ok(XmlEvent::End(ref e)) => {
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == parent_tag {
                        break;
                    }
                }
                Ok(XmlEvent::Eof) => break,
                _ => {}
            }
            buf.clear();
        }
    }

    fn parse_extension_elements_base(
        &self,
        reader: &mut Reader<&[u8]>,
        element: &mut FlowElement,
        namespaces: &IndexMap<String, String>,
    ) {
        let mut buf = Vec::new();
        while let Ok(event) = reader.read_event_into(&mut buf) {
            match event {
                XmlEvent::Start(ref e) | XmlEvent::Empty(ref e) => {
                    let is_empty = matches!(event, XmlEvent::Empty(_));
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == ELEMENT_EXECUTION_LISTENER {
                        element
                            .execution_listeners
                            .push(self.parse_execution_listener(e, reader, is_empty));
                    } else if local_name == "customResource"
                        || local_name == "taskListener"
                        || local_name == "formProperty"
                        || local_name == "value"
                    {
                        if !is_empty {
                            let _ = self.read_element_text(reader, e.name());
                        }
                    } else {
                        let ext =
                            self.parse_generic_extension_element(e, reader, namespaces, is_empty);
                        element
                            .base_element
                            .extension_elements
                            .entry(local_name)
                            .or_default()
                            .push(ext);
                    }
                }
                XmlEvent::End(ref e) => {
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == "extensionElements" {
                        break;
                    }
                }
                XmlEvent::Eof => break,
                _ => {}
            }
            buf.clear();
        }
    }

    fn read_element_text(
        &self,
        reader: &mut Reader<&[u8]>,
        name: quick_xml::name::QName,
    ) -> String {
        let text = reader.read_text(name).unwrap().into_owned();
        text.replace("<![CDATA[", "")
            .replace("]]>", "")
            .replace("\r\n", "\n")
    }

    fn skip_element(&self, reader: &mut Reader<&[u8]>, _name: quick_xml::name::QName) {
        let mut buf = Vec::new();
        let mut depth = 1;
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(XmlEvent::Start(_)) => depth += 1,
                Ok(XmlEvent::End(_)) => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                Ok(XmlEvent::Eof) => break,
                _ => {}
            }
            buf.clear();
        }
    }

    fn read_element_text_preserve_whitespace(
        &self,
        reader: &mut Reader<&[u8]>,
        name: quick_xml::name::QName,
    ) -> String {
        reader.config_mut().trim_text(false);
        let text = reader.read_text(name).unwrap().into_owned();
        reader.config_mut().trim_text(true);
        text.replace("<![CDATA[", "")
            .replace("]]>", "")
            .replace("\r\n", "\n")
            .trim()
            .to_string()
    }
    fn parse_di(&self, _e: &BytesStart, reader: &mut Reader<&[u8]>, model: &mut BpmnModel) {
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(XmlEvent::Start(ref inner_e)) => {
                    let local_name =
                        self.get_local_name_bytes(inner_e.local_name().as_ref(), reader);
                    if local_name == ELEMENT_DI_PLANE {
                        self.parse_di_plane(inner_e, reader, model);
                    }
                }
                Ok(XmlEvent::End(ref inner_e)) => {
                    let local_name =
                        self.get_local_name_bytes(inner_e.local_name().as_ref(), reader);
                    if local_name == ELEMENT_DI_DIAGRAM {
                        break;
                    }
                }
                Ok(XmlEvent::Eof) => break,
                _ => {}
            }
            buf.clear();
        }
    }

    fn parse_di_plane(&self, _e: &BytesStart, reader: &mut Reader<&[u8]>, model: &mut BpmnModel) {
        let mut buf = Vec::new();
        while let Ok(event) = reader.read_event_into(&mut buf) {
            match event {
                XmlEvent::Start(ref e) | XmlEvent::Empty(ref e) => {
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == ELEMENT_DI_SHAPE {
                        self.parse_di_shape(e, reader, model);
                    } else if local_name == ELEMENT_DI_EDGE {
                        self.parse_di_edge(e, reader, model);
                    }
                }
                XmlEvent::End(ref e) => {
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == ELEMENT_DI_PLANE {
                        break;
                    }
                }
                XmlEvent::Eof => break,
                _ => {}
            }
            buf.clear();
        }
    }

    fn parse_di_shape(&self, e: &BytesStart, reader: &mut Reader<&[u8]>, model: &mut BpmnModel) {
        let offset = reader.buffer_position();
        let (row, col) = self.get_position(reader, offset as usize);
        let mut graphic_info = GraphicInfo::default();
        graphic_info.xml_row_number = row;
        graphic_info.xml_column_number = col;
        let mut bpmn_element = String::new();
        for attr in e.attributes() {
            let Ok(attr) = attr else {
                continue;
            };
            let local_key = self.get_local_name_bytes(attr.key.as_ref(), reader);
            let value = attr
                .decode_and_unescape_value(reader.decoder())
                .unwrap_or_default();
            match local_key.as_str() {
                k if k == ATTRIBUTE_DI_BPMNELEMENT => bpmn_element = value.into_owned(),
                k if k == ATTRIBUTE_DI_IS_EXPANDED => {
                    graphic_info.expanded = Some(value == ATTRIBUTE_VALUE_TRUE)
                }
                _ => {}
            }
        }

        let mut inner_buf = Vec::new();
        let mut is_in_label = false;
        let mut label_graphic_info = GraphicInfo::default();
        loop {
            let event = reader.read_event_into(&mut inner_buf).unwrap();
            match event {
                XmlEvent::Start(ref inner_e) | XmlEvent::Empty(ref inner_e) => {
                    let inner_name =
                        self.get_local_name_bytes(inner_e.local_name().as_ref(), reader);
                    if inner_name == "BPMNLabel" {
                        is_in_label = true;
                        label_graphic_info.xml_row_number = row;
                        label_graphic_info.xml_column_number = col;
                    } else if inner_name == ELEMENT_DI_BOUNDS {
                        for attr in inner_e.attributes() {
                            let Ok(attr) = attr else {
                                continue;
                            };
                            let local_key = self.get_local_name_bytes(attr.key.as_ref(), reader);
                            let value = attr
                                .decode_and_unescape_value(reader.decoder())
                                .unwrap_or_default();
                            let val = value.parse::<f64>().unwrap_or(0.0);
                            match local_key.as_str() {
                                k if k == ATTRIBUTE_DI_X => {
                                    if is_in_label {
                                        label_graphic_info.x = val
                                    } else {
                                        graphic_info.x = val
                                    }
                                }
                                k if k == ATTRIBUTE_DI_Y => {
                                    if is_in_label {
                                        label_graphic_info.y = val
                                    } else {
                                        graphic_info.y = val
                                    }
                                }
                                k if k == ATTRIBUTE_DI_WIDTH => {
                                    if is_in_label {
                                        label_graphic_info.width = val
                                    } else {
                                        graphic_info.width = val
                                    }
                                }
                                k if k == ATTRIBUTE_DI_HEIGHT => {
                                    if is_in_label {
                                        label_graphic_info.height = val
                                    } else {
                                        graphic_info.height = val
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
                XmlEvent::End(ref inner_e) => {
                    let inner_name =
                        self.get_local_name_bytes(inner_e.local_name().as_ref(), reader);
                    if inner_name == "BPMNLabel" {
                        is_in_label = false;
                        if !bpmn_element.is_empty() {
                            model
                                .label_location_map
                                .insert(bpmn_element.clone(), label_graphic_info.clone());
                        }
                    } else if inner_name == ELEMENT_DI_SHAPE {
                        break;
                    }
                }
                XmlEvent::Eof => break,
                _ => {}
            }
            inner_buf.clear();
        }
        model.location_map.insert(bpmn_element, graphic_info);
    }

    fn parse_di_edge(&self, e: &BytesStart, reader: &mut Reader<&[u8]>, model: &mut BpmnModel) {
        let mut waypoints = Vec::new();
        let mut bpmn_element = String::new();
        for attr in e.attributes() {
            let Ok(attr) = attr else {
                continue;
            };
            let local_key = self.get_local_name_bytes(attr.key.as_ref(), reader);
            let value = attr
                .decode_and_unescape_value(reader.decoder())
                .unwrap_or_default();
            if local_key == ATTRIBUTE_DI_BPMNELEMENT {
                bpmn_element = value.into_owned();
            }
        }

        let mut buf = Vec::new();
        while let Ok(event) = reader.read_event_into(&mut buf) {
            match event {
                XmlEvent::Start(ref e) | XmlEvent::Empty(ref e) => {
                    let inner_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if inner_name == ELEMENT_DI_WAYPOINT {
                        let mut gi = GraphicInfo::default();
                        let inner_offset = reader.buffer_position();
                        let (i_row, i_col) = self.get_position(reader, inner_offset as usize);
                        gi.xml_row_number = i_row;
                        gi.xml_column_number = i_col;
                        for attr in e.attributes() {
                            let Ok(attr) = attr else {
                                continue;
                            };
                            let local_key = self.get_local_name_bytes(attr.key.as_ref(), reader);
                            let value = attr
                                .decode_and_unescape_value(reader.decoder())
                                .unwrap_or_default();
                            let val = value.parse::<f64>().unwrap_or(0.0).trunc();
                            match local_key.as_str() {
                                k if k == ATTRIBUTE_DI_X => gi.x = val,
                                k if k == ATTRIBUTE_DI_Y => gi.y = val,
                                _ => {}
                            }
                        }
                        waypoints.push(gi);
                    }
                }
                XmlEvent::End(ref e) => {
                    let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
                    if local_name == ELEMENT_DI_EDGE {
                        break;
                    }
                }
                XmlEvent::Eof => {
                    tracing::warn!("EOF reached while parsing BPMNEdge for {}", bpmn_element);
                    break;
                }
                _ => {}
            }
            buf.clear();
        }
        model.flow_location_map.insert(bpmn_element, waypoints);
    }

    fn get_position(&self, reader: &Reader<&[u8]>, offset: usize) -> (i32, i32) {
        let mut line = 1;
        let mut column = 1;
        let bytes = reader.get_ref();
        for (i, &b) in bytes.iter().enumerate() {
            if i >= offset {
                break;
            }
            if b == b'\n' {
                line += 1;
                column = 1;
            } else {
                column += 1;
            }
        }
        (line, column)
    }

    fn post_process_process(&self, process: &mut Process, errors: &HashMap<String, String>) {
        self.resolve_error_codes(&mut process.flow_elements, errors);
        self.process_flow_elements_scope(&mut process.flow_elements);
        self.populate_all_maps(process);
    }

    fn process_flow_elements_scope(&self, flow_elements: &mut [FlowElementEnum]) {
        let mut sequence_flows = Vec::new();

        for element in flow_elements.iter() {
            if let FlowElementEnum::SequenceFlow(sf) = element {
                sequence_flows.push(sf.clone());
            }
        }

        for sf in sequence_flows {
            if let Some(source_ref) = &sf.source_ref {
                self.add_outgoing_flow_local(flow_elements, source_ref, &sf);
            }
            if let Some(target_ref) = &sf.target_ref {
                self.add_incoming_flow_local(flow_elements, target_ref, &sf);
            }
        }

        let mut boundary_events = Vec::new();
        for element in flow_elements.iter() {
            if let FlowElementEnum::BoundaryEvent(be) = element {
                boundary_events.push(be.clone());
            }
        }

        for be in boundary_events {
            if let Some(ref attached_ref) = be.attached_to_ref_id {
                self.add_boundary_event_local(flow_elements, attached_ref, &be);
            }
        }

        for element in flow_elements.iter_mut() {
            if let Some(sub_proc) = self.get_sub_process_mut(element) {
                self.process_flow_elements_scope(&mut sub_proc.flow_elements);
            }
        }
    }

    #[allow(clippy::collapsible_if)]
    fn add_outgoing_flow_local(
        &self,
        flow_elements: &mut [FlowElementEnum],
        node_id: &str,
        sf: &SequenceFlow,
    ) {
        for element in flow_elements.iter_mut().rev() {
            if let Some(id) = self.get_element_id(element) {
                if id == node_id {
                    if let Some(flow_node) = self.get_flow_node_mut(element) {
                        flow_node.outgoing_flows.push(sf.clone());
                        return;
                    }
                }
            }
        }
    }

    #[allow(clippy::collapsible_if)]
    fn add_incoming_flow_local(
        &self,
        flow_elements: &mut [FlowElementEnum],
        node_id: &str,
        sf: &SequenceFlow,
    ) {
        for element in flow_elements.iter_mut().rev() {
            if let Some(id) = self.get_element_id(element) {
                if id == node_id {
                    if let Some(flow_node) = self.get_flow_node_mut(element) {
                        flow_node.incoming_flows.push(sf.clone());
                        return;
                    }
                }
            }
        }
    }

    #[allow(clippy::collapsible_if)]
    fn add_boundary_event_local(
        &self,
        flow_elements: &mut [FlowElementEnum],
        activity_id: &str,
        be: &BoundaryEvent,
    ) {
        for element in flow_elements.iter_mut().rev() {
            if let Some(id) = self.get_element_id(element) {
                if id == activity_id {
                    if let Some(activity) = self.get_activity_mut(element) {
                        activity.boundary_events.push(be.clone());
                        return;
                    }
                }
            }
        }
    }

    #[allow(clippy::collapsible_if)]
    fn resolve_error_codes(
        &self,
        flow_elements: &mut [FlowElementEnum],
        errors: &HashMap<String, String>,
    ) {
        for element in flow_elements {
            match element {
                FlowElementEnum::BoundaryEvent(be) => {
                    for ed in &mut be.event.event_definitions {
                        if let EventDefinitionEnum::ErrorEventDefinition(eed) = ed {
                            if let Some(ref error_ref) = eed.error_ref {
                                if let Some(error_code) = errors.get(error_ref) {
                                    eed.error_code = Some(error_code.clone());
                                }
                            }
                        }
                    }
                }
                FlowElementEnum::SubProcess(s) => {
                    self.resolve_error_codes(&mut s.flow_elements, errors)
                }
                _ => {}
            }
        }
    }

    fn get_activity_mut<'a>(&self, element: &'a mut FlowElementEnum) -> Option<&'a mut Activity> {
        match element {
            FlowElementEnum::Task(e) => Some(&mut e.activity),
            FlowElementEnum::UserTask(e) => Some(&mut e.task.activity),
            FlowElementEnum::ServiceTask(e) => Some(&mut e.task.activity),
            FlowElementEnum::CaseServiceTask(e) => Some(&mut e.service_task.task.activity),
            FlowElementEnum::SendTask(e) => Some(&mut e.service_task.task.activity),
            FlowElementEnum::ScriptTask(e) => Some(&mut e.task.activity),
            FlowElementEnum::ManualTask(e) => Some(&mut e.task.activity),
            FlowElementEnum::ReceiveTask(e) => Some(&mut e.task.activity),
            FlowElementEnum::BusinessRuleTask(e) => Some(&mut e.task.activity),
            FlowElementEnum::SubProcess(e) => Some(&mut e.activity),
            FlowElementEnum::Transaction(e) => Some(&mut e.sub_process.activity),
            FlowElementEnum::EventSubProcess(e) => Some(&mut e.sub_process.activity),
            FlowElementEnum::AdhocSubProcess(e) => Some(&mut e.sub_process.activity),
            FlowElementEnum::CallActivity(e) => Some(&mut e.activity),
            _ => None,
        }
    }

    fn populate_all_maps(&self, process: &mut Process) {
        for element in &mut process.flow_elements {
            self.populate_sub_process_maps_recursive(element, 0);
        }

        let mut map = IndexMap::new();
        self.populate_element_map(&process.flow_elements, &mut map, &process.data_objects, 0);
        process.flow_element_map = map;
        process.artifact_map =
            self.populate_artifact_map(&process.artifacts, &process.flow_elements, 0);
    }

    fn populate_sub_process_maps_recursive(&self, element: &mut FlowElementEnum, depth: usize) {
        if depth >= MAX_XML_NESTING_DEPTH {
            // M3: refuse to descend past the nesting cap even for
            // programmatically-built models; XML-parsed models cannot reach
            // this because validate_well_formed_xml caps their nesting.
            return;
        }
        if let Some(sub_proc) = self.get_sub_process_mut(element) {
            for child in &mut sub_proc.flow_elements {
                self.populate_sub_process_maps_recursive(child, depth + 1);
            }

            let mut map = IndexMap::new();
            self.populate_element_map(&sub_proc.flow_elements, &mut map, &[], depth + 1);
            sub_proc.flow_element_map = map;
            sub_proc.artifact_map =
                self.populate_artifact_map(&sub_proc.artifacts, &sub_proc.flow_elements, depth + 1);
        }
    }

    fn populate_artifact_map(
        &self,
        artifacts: &[ArtifactEnum],
        flow_elements: &[FlowElementEnum],
        depth: usize,
    ) -> IndexMap<String, ArtifactEnum> {
        let mut map = IndexMap::new();
        self.insert_artifacts(artifacts, &mut map);
        self.insert_nested_artifacts(flow_elements, &mut map, depth);
        map
    }

    fn insert_artifacts(
        &self,
        artifacts: &[ArtifactEnum],
        map: &mut IndexMap<String, ArtifactEnum>,
    ) {
        for artifact in artifacts {
            let id = match artifact {
                ArtifactEnum::Association(value) => value.base_element.id.as_ref(),
                ArtifactEnum::TextAnnotation(value) => value.base_element.id.as_ref(),
                ArtifactEnum::Group(value) => value.base_element.id.as_ref(),
            };
            if let Some(id) = id {
                map.insert(id.clone(), artifact.clone());
            }
        }
    }

    fn insert_nested_artifacts(
        &self,
        flow_elements: &[FlowElementEnum],
        map: &mut IndexMap<String, ArtifactEnum>,
        depth: usize,
    ) {
        if depth >= MAX_XML_NESTING_DEPTH {
            return;
        }
        for element in flow_elements {
            let sub_process = match element {
                FlowElementEnum::SubProcess(value) => Some(value),
                FlowElementEnum::Transaction(value) => Some(&value.sub_process),
                FlowElementEnum::EventSubProcess(value) => Some(&value.sub_process),
                FlowElementEnum::AdhocSubProcess(value) => Some(&value.sub_process),
                _ => None,
            };
            if let Some(sub_process) = sub_process {
                self.insert_artifacts(&sub_process.artifacts, map);
                self.insert_nested_artifacts(&sub_process.flow_elements, map, depth + 1);
            }
        }
    }

    #[allow(clippy::collapsible_if)]
    fn populate_element_map(
        &self,
        flow_elements: &[FlowElementEnum],
        map: &mut IndexMap<String, FlowElementEnum>,
        data_objects: &[ValuedDataObject],
        depth: usize,
    ) {
        if depth >= MAX_XML_NESTING_DEPTH {
            return;
        }
        for element in flow_elements {
            if let Some(id) = self.get_element_id(element) {
                map.insert(id, element.clone());
            }
            match element {
                FlowElementEnum::SubProcess(s) => {
                    self.populate_element_map(&s.flow_elements, map, &[], depth + 1)
                }
                FlowElementEnum::Transaction(t) => {
                    self.populate_element_map(&t.sub_process.flow_elements, map, &[], depth + 1)
                }
                FlowElementEnum::EventSubProcess(e) => {
                    self.populate_element_map(&e.sub_process.flow_elements, map, &[], depth + 1)
                }
                FlowElementEnum::AdhocSubProcess(a) => {
                    self.populate_element_map(&a.sub_process.flow_elements, map, &[], depth + 1)
                }
                _ => {}
            }
        }
        for obj in data_objects {
            if let Some(ref id) = obj.base_element.id {
                if !map.contains_key(id) {
                    map.insert(id.clone(), FlowElementEnum::ValuedDataObject(obj.clone()));
                }
            }
        }
    }

    fn get_flow_node_mut<'a>(&self, element: &'a mut FlowElementEnum) -> Option<&'a mut FlowNode> {
        match element {
            FlowElementEnum::UserTask(e) => Some(&mut e.task.activity.flow_node),
            FlowElementEnum::ServiceTask(e) => Some(&mut e.task.activity.flow_node),
            FlowElementEnum::CaseServiceTask(e) => {
                Some(&mut e.service_task.task.activity.flow_node)
            }
            FlowElementEnum::SendTask(e) => Some(&mut e.service_task.task.activity.flow_node),
            FlowElementEnum::ScriptTask(e) => Some(&mut e.task.activity.flow_node),
            FlowElementEnum::ManualTask(e) => Some(&mut e.task.activity.flow_node),
            FlowElementEnum::ReceiveTask(e) => Some(&mut e.task.activity.flow_node),
            FlowElementEnum::BusinessRuleTask(e) => Some(&mut e.task.activity.flow_node),
            FlowElementEnum::StartEvent(e) => Some(&mut e.event.flow_node),
            FlowElementEnum::EndEvent(e) => Some(&mut e.event.flow_node),
            FlowElementEnum::ExclusiveGateway(e) => Some(&mut e.gateway.flow_node),
            FlowElementEnum::ParallelGateway(e) => Some(&mut e.gateway.flow_node),
            FlowElementEnum::InclusiveGateway(e) => Some(&mut e.gateway.flow_node),
            FlowElementEnum::EventBasedGateway(e) => Some(&mut e.gateway.flow_node),
            FlowElementEnum::ComplexGateway(e) => Some(&mut e.gateway.flow_node),
            FlowElementEnum::IntermediateCatchEvent(e) => Some(&mut e.event.flow_node),
            FlowElementEnum::IntermediateThrowEvent(e) => Some(&mut e.event.flow_node),
            FlowElementEnum::BoundaryEvent(e) => Some(&mut e.event.flow_node),
            FlowElementEnum::SubProcess(e) => Some(&mut e.activity.flow_node),
            FlowElementEnum::Transaction(e) => Some(&mut e.sub_process.activity.flow_node),
            FlowElementEnum::EventSubProcess(e) => Some(&mut e.sub_process.activity.flow_node),
            FlowElementEnum::AdhocSubProcess(e) => Some(&mut e.sub_process.activity.flow_node),
            FlowElementEnum::CallActivity(e) => Some(&mut e.activity.flow_node),
            _ => None,
        }
    }

    fn get_sub_process_mut<'a>(
        &self,
        element: &'a mut FlowElementEnum,
    ) -> Option<&'a mut SubProcess> {
        match element {
            FlowElementEnum::SubProcess(s) => Some(s),
            FlowElementEnum::Transaction(t) => Some(&mut t.sub_process),
            FlowElementEnum::EventSubProcess(e) => Some(&mut e.sub_process),
            FlowElementEnum::AdhocSubProcess(a) => Some(&mut a.sub_process),
            _ => None,
        }
    }

    fn get_element_id(&self, element: &FlowElementEnum) -> Option<String> {
        match element {
            FlowElementEnum::SequenceFlow(e) => e.flow_element.base_element.id.clone(),
            FlowElementEnum::Task(e) => e.activity.flow_node.flow_element.base_element.id.clone(),
            FlowElementEnum::UserTask(e) => e
                .task
                .activity
                .flow_node
                .flow_element
                .base_element
                .id
                .clone(),
            FlowElementEnum::ServiceTask(e) => e
                .task
                .activity
                .flow_node
                .flow_element
                .base_element
                .id
                .clone(),
            FlowElementEnum::CaseServiceTask(e) => e
                .service_task
                .task
                .activity
                .flow_node
                .flow_element
                .base_element
                .id
                .clone(),
            FlowElementEnum::SendTask(e) => e
                .service_task
                .task
                .activity
                .flow_node
                .flow_element
                .base_element
                .id
                .clone(),
            FlowElementEnum::ScriptTask(e) => e
                .task
                .activity
                .flow_node
                .flow_element
                .base_element
                .id
                .clone(),
            FlowElementEnum::ManualTask(e) => e
                .task
                .activity
                .flow_node
                .flow_element
                .base_element
                .id
                .clone(),
            FlowElementEnum::ReceiveTask(e) => e
                .task
                .activity
                .flow_node
                .flow_element
                .base_element
                .id
                .clone(),
            FlowElementEnum::BusinessRuleTask(e) => e
                .task
                .activity
                .flow_node
                .flow_element
                .base_element
                .id
                .clone(),
            FlowElementEnum::StartEvent(e) => {
                e.event.flow_node.flow_element.base_element.id.clone()
            }
            FlowElementEnum::EndEvent(e) => e.event.flow_node.flow_element.base_element.id.clone(),
            FlowElementEnum::ExclusiveGateway(e) => {
                e.gateway.flow_node.flow_element.base_element.id.clone()
            }
            FlowElementEnum::ParallelGateway(e) => {
                e.gateway.flow_node.flow_element.base_element.id.clone()
            }
            FlowElementEnum::InclusiveGateway(e) => {
                e.gateway.flow_node.flow_element.base_element.id.clone()
            }
            FlowElementEnum::EventBasedGateway(e) => {
                e.gateway.flow_node.flow_element.base_element.id.clone()
            }
            FlowElementEnum::ComplexGateway(e) => {
                e.gateway.flow_node.flow_element.base_element.id.clone()
            }
            FlowElementEnum::IntermediateCatchEvent(e) => {
                e.event.flow_node.flow_element.base_element.id.clone()
            }
            FlowElementEnum::IntermediateThrowEvent(e) => {
                e.event.flow_node.flow_element.base_element.id.clone()
            }
            FlowElementEnum::BoundaryEvent(e) => {
                e.event.flow_node.flow_element.base_element.id.clone()
            }
            FlowElementEnum::SubProcess(e) => {
                e.activity.flow_node.flow_element.base_element.id.clone()
            }
            FlowElementEnum::Transaction(e) => e
                .sub_process
                .activity
                .flow_node
                .flow_element
                .base_element
                .id
                .clone(),
            FlowElementEnum::EventSubProcess(e) => e
                .sub_process
                .activity
                .flow_node
                .flow_element
                .base_element
                .id
                .clone(),
            FlowElementEnum::AdhocSubProcess(e) => e
                .sub_process
                .activity
                .flow_node
                .flow_element
                .base_element
                .id
                .clone(),
            FlowElementEnum::CallActivity(e) => {
                e.activity.flow_node.flow_element.base_element.id.clone()
            }
            FlowElementEnum::ValuedDataObject(e) => e.base_element.id.clone(),
        }
    }

    fn parse_common_flow_node_attributes(
        &self,
        e: &BytesStart,
        reader: &Reader<&[u8]>,
        flow_node: &mut FlowNode,
    ) {
        flow_node.exclusive = true;
        flow_node.asynchronous_leave_exclusive = true;
        for attr in e.attributes() {
            let Ok(attr) = attr else {
                continue;
            };
            let local_key = self.get_local_name_bytes(attr.key.as_ref(), reader);
            let value = attr
                .decode_and_unescape_value(reader.decoder())
                .unwrap_or_default();
            match local_key.as_str() {
                k if k == ATTRIBUTE_ID => {
                    flow_node.flow_element.base_element.id = Some(value.into_owned())
                }
                k if k == ATTRIBUTE_NAME => flow_node.flow_element.name = Some(value.into_owned()),
                k if k == ATTRIBUTE_ACTIVITY_ASYNCHRONOUS => {
                    flow_node.asynchronous = value == ATTRIBUTE_VALUE_TRUE
                }
                k if k == ATTRIBUTE_ACTIVITY_EXCLUSIVE => {
                    flow_node.exclusive = value != ATTRIBUTE_VALUE_FALSE;
                    flow_node.not_exclusive = value == ATTRIBUTE_VALUE_FALSE;
                }
                k if k == ATTRIBUTE_ACTIVITY_ASYNCHRONOUS_LEAVE => {
                    flow_node.asynchronous_leave = value == ATTRIBUTE_VALUE_TRUE
                }
                k if k == ATTRIBUTE_ACTIVITY_ASYNCHRONOUS_LEAVE_EXCLUSIVE => {
                    flow_node.asynchronous_leave_exclusive = value != ATTRIBUTE_VALUE_FALSE;
                    flow_node.asynchronous_leave_not_exclusive = value == ATTRIBUTE_VALUE_FALSE;
                }
                _ => {}
            }
        }
    }

    fn parse_artifact(
        &self,
        e: &BytesStart,
        reader: &mut Reader<&[u8]>,
        is_empty: bool,
        namespaces: &IndexMap<String, String>,
    ) -> Option<ArtifactEnum> {
        let local_name = self.get_local_name_bytes(e.local_name().as_ref(), reader);
        match local_name.as_str() {
            "association" => Some(ArtifactEnum::Association(
                self.parse_association(e, reader, is_empty, namespaces),
            )),
            ELEMENT_TEXT_ANNOTATION => Some(ArtifactEnum::TextAnnotation(
                self.parse_text_annotation(e, reader, is_empty, namespaces),
            )),
            ELEMENT_GROUP => Some(ArtifactEnum::Group(
                self.parse_group(e, reader, is_empty, namespaces),
            )),
            _ => None,
        }
    }

    fn parse_association(
        &self,
        e: &BytesStart,
        reader: &mut Reader<&[u8]>,
        is_empty: bool,
        namespaces: &IndexMap<String, String>,
    ) -> Association {
        let mut association = Association::default();
        let offset = reader.buffer_position();
        let (row, col) = self.get_position(reader, offset as usize);
        association.base_element.xml_row_number = row;
        association.base_element.xml_column_number = col;
        for attr in e.attributes().flatten() {
            let key = reader
                .decoder()
                .decode(attr.key.as_ref())
                .unwrap_or_default();
            let local_key = self.get_local_name_bytes(attr.key.as_ref(), reader);
            let value = attr
                .decode_and_unescape_value(reader.decoder())
                .unwrap_or_default();
            match local_key.as_str() {
                ATTRIBUTE_ID => association.base_element.id = Some(value.into_owned()),
                "sourceRef" => association.source_ref = Some(value.into_owned()),
                "targetRef" => association.target_ref = Some(value.into_owned()),
                "associationDirection" => {
                    association.association_direction = Some(
                        match value.as_ref().to_ascii_uppercase().as_str() {
                            "ONE" => "ONE",
                            "BOTH" => "BOTH",
                            "NONE" => "NONE",
                            _ => value.as_ref(),
                        }
                        .to_string(),
                    )
                }
                _ => self.store_artifact_attribute(
                    &mut association.base_element,
                    key.as_ref(),
                    local_key,
                    value.into_owned(),
                    namespaces,
                ),
            }
        }
        self.ensure_id(&mut association.base_element.id);

        if !is_empty {
            self.parse_artifact_extension_children(
                reader,
                &mut association.base_element,
                "association",
                namespaces,
            );
        }

        association
    }

    fn parse_text_annotation(
        &self,
        e: &BytesStart,
        reader: &mut Reader<&[u8]>,
        is_empty: bool,
        namespaces: &IndexMap<String, String>,
    ) -> TextAnnotation {
        let mut annotation = TextAnnotation::default();
        let offset = reader.buffer_position();
        let (row, col) = self.get_position(reader, offset as usize);
        annotation.base_element.xml_row_number = row;
        annotation.base_element.xml_column_number = col;
        for attr in e.attributes().flatten() {
            let key = reader
                .decoder()
                .decode(attr.key.as_ref())
                .unwrap_or_default();
            let local_key = self.get_local_name_bytes(attr.key.as_ref(), reader);
            let value = attr
                .decode_and_unescape_value(reader.decoder())
                .unwrap_or_default();
            match local_key.as_str() {
                ATTRIBUTE_ID => annotation.base_element.id = Some(value.into_owned()),
                "textFormat" => annotation.text_format = Some(value.into_owned()),
                _ => self.store_artifact_attribute(
                    &mut annotation.base_element,
                    key.as_ref(),
                    local_key,
                    value.into_owned(),
                    namespaces,
                ),
            }
        }
        self.ensure_id(&mut annotation.base_element.id);

        if !is_empty {
            let mut buf = Vec::new();
            loop {
                match reader.read_event_into(&mut buf) {
                    Ok(XmlEvent::Start(ref inner_e)) => {
                        let inner_name =
                            self.get_local_name_bytes(inner_e.local_name().as_ref(), reader);
                        if inner_name == "text" {
                            annotation.text = Some(self.read_element_text(reader, inner_e.name()));
                        } else if inner_name == "extensionElements" {
                            self.parse_generic_extension_elements_into_base_element(
                                reader,
                                &mut annotation.base_element,
                                inner_e,
                                namespaces,
                            );
                        }
                    }
                    Ok(XmlEvent::End(ref inner_e)) => {
                        let inner_name =
                            self.get_local_name_bytes(inner_e.local_name().as_ref(), reader);
                        if inner_name == ELEMENT_TEXT_ANNOTATION {
                            break;
                        }
                    }
                    Ok(XmlEvent::Eof) => break,
                    _ => {}
                }
                buf.clear();
            }
        }

        annotation
    }

    fn parse_group(
        &self,
        e: &BytesStart,
        reader: &mut Reader<&[u8]>,
        is_empty: bool,
        namespaces: &IndexMap<String, String>,
    ) -> Group {
        let mut group = Group::default();
        let offset = reader.buffer_position();
        let (row, col) = self.get_position(reader, offset as usize);
        group.base_element.xml_row_number = row;
        group.base_element.xml_column_number = col;
        for attr in e.attributes().flatten() {
            let key = reader
                .decoder()
                .decode(attr.key.as_ref())
                .unwrap_or_default();
            let local_key = self.get_local_name_bytes(attr.key.as_ref(), reader);
            let value = attr
                .decode_and_unescape_value(reader.decoder())
                .unwrap_or_default();
            match local_key.as_str() {
                ATTRIBUTE_ID => group.base_element.id = Some(value.into_owned()),
                "categoryValueRef" => group.category_value_ref = Some(value.into_owned()),
                _ => self.store_artifact_attribute(
                    &mut group.base_element,
                    key.as_ref(),
                    local_key,
                    value.into_owned(),
                    namespaces,
                ),
            }
        }
        self.ensure_id(&mut group.base_element.id);

        if !is_empty {
            self.parse_artifact_extension_children(
                reader,
                &mut group.base_element,
                ELEMENT_GROUP,
                namespaces,
            );
        }
        group
    }

    fn parse_artifact_extension_children(
        &self,
        reader: &mut Reader<&[u8]>,
        base_element: &mut BaseElement,
        parent_tag: &str,
        namespaces: &IndexMap<String, String>,
    ) {
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(XmlEvent::Start(ref inner_e)) => {
                    let inner_name =
                        self.get_local_name_bytes(inner_e.local_name().as_ref(), reader);
                    if inner_name == "extensionElements" {
                        self.parse_generic_extension_elements_into_base_element(
                            reader,
                            base_element,
                            inner_e,
                            namespaces,
                        );
                    }
                }
                Ok(XmlEvent::End(ref inner_e)) => {
                    let inner_name =
                        self.get_local_name_bytes(inner_e.local_name().as_ref(), reader);
                    if inner_name == parent_tag {
                        break;
                    }
                }
                Ok(XmlEvent::Eof) => break,
                _ => {}
            }
            buf.clear();
        }
    }

    fn store_artifact_attribute(
        &self,
        base_element: &mut BaseElement,
        qualified_name: &str,
        local_name: String,
        value: String,
        namespaces: &IndexMap<String, String>,
    ) {
        let mut attribute = ExtensionAttribute {
            name: Some(local_name.clone()),
            value: Some(value),
            ..ExtensionAttribute::default()
        };
        if let Some((prefix, _)) = qualified_name.split_once(':') {
            attribute.namespace_prefix = Some(prefix.to_string());
            attribute.namespace = namespaces.get(prefix).cloned();
        }
        base_element
            .attributes
            .entry(local_name)
            .or_default()
            .push(attribute);
    }

    fn parse_common_activity_attributes(
        &self,
        e: &BytesStart,
        reader: &Reader<&[u8]>,
        activity: &mut Activity,
    ) {
        self.parse_common_flow_node_attributes(e, reader, &mut activity.flow_node);
        for attr in e.attributes().flatten() {
            let local_key = self.get_local_name_bytes(attr.key.as_ref(), reader);
            let value = attr
                .decode_and_unescape_value(reader.decoder())
                .unwrap_or_default();
            match local_key.as_str() {
                "isForCompensation" => {
                    let is_for_compensation = value == ATTRIBUTE_VALUE_TRUE;
                    activity.is_for_compensation = is_for_compensation;
                    activity.for_compensation = is_for_compensation;
                }
                "default" => activity.default_flow = Some(value.into_owned()),
                _ => {}
            }
        }
    }

    fn parse_gateway_attributes(
        &self,
        e: &BytesStart,
        reader: &Reader<&[u8]>,
        gateway: &mut Gateway,
    ) {
        for attr in e.attributes() {
            let Ok(attr) = attr else {
                continue;
            };
            let local_key = self.get_local_name_bytes(attr.key.as_ref(), reader);
            let value = attr
                .decode_and_unescape_value(reader.decoder())
                .unwrap_or_default();
            if local_key == ATTRIBUTE_DEFAULT {
                gateway.default_flow = Some(value.into_owned());
            }
        }
    }

    fn get_local_name_bytes(&self, bytes: &[u8], reader: &Reader<&[u8]>) -> String {
        let name = reader.decoder().decode(bytes).unwrap_or_default();
        if let Some(pos) = name.find(':') {
            name[pos + 1..].to_string()
        } else {
            name.into_owned()
        }
    }
}
