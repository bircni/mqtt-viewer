//! Protobuf decoding from `.proto` files loaded at runtime, plus the built-in Sparkplug B schema.

use std::path::{Path, PathBuf};

use prost_reflect::{DescriptorPool, DynamicMessage, MessageDescriptor, SerializeOptions};
use protox::file::{File, FileResolver};
use serde_json::Value;

use crate::config::ProtoBinding;

const SPARKPLUG_FILE: &str = "sparkplug_b.proto";
const SPARKPLUG_SOURCE: &str = include_str!("sparkplug_b.proto");
const SPARKPLUG_MESSAGE: &str = "org.eclipse.tahu.protobuf.Payload";

/// Serves the embedded Sparkplug B schema to the compiler.
struct EmbeddedSparkplug;

impl FileResolver for EmbeddedSparkplug {
    fn open_file(&self, name: &str) -> Result<File, protox::Error> {
        if name == SPARKPLUG_FILE {
            File::from_source(name, SPARKPLUG_SOURCE)
        } else {
            Err(protox::Error::file_not_found(name))
        }
    }
}

/// The Protobuf message types known to the decoder and the topics they are bound to.
#[derive(Debug, Clone, Default)]
pub struct ProtoRegistry {
    pool: DescriptorPool,
    bindings: Vec<ProtoBinding>,
    sparkplug: Option<MessageDescriptor>,
}

impl ProtoRegistry {
    /// A registry knowing only the built-in Sparkplug B schema.
    #[must_use]
    pub fn new() -> Self {
        let mut compiler = protox::Compiler::with_file_resolver(EmbeddedSparkplug);
        let sparkplug = compiler
            .open_file(SPARKPLUG_FILE)
            .ok()
            .and_then(|compiler| compiler.descriptor_pool().get_message_by_name(SPARKPLUG_MESSAGE));
        Self { pool: DescriptorPool::new(), bindings: Vec::new(), sparkplug }
    }

    /// Compiles `files` and registers `bindings`. Files that fail to compile are skipped and reported in the
    /// returned list, so one broken schema does not disable the others.
    #[must_use]
    pub fn load(files: &[PathBuf], bindings: &[ProtoBinding]) -> (Self, Vec<String>) {
        let mut registry = Self::new();
        let mut errors = Vec::new();
        for file in files {
            match compile(file) {
                Ok(pool) => {
                    for descriptor in pool.file_descriptor_protos() {
                        if registry.pool.get_file_by_name(descriptor.name()).is_none()
                            && let Err(error) = registry.pool.add_file_descriptor_proto(descriptor.clone())
                        {
                            errors.push(format!("{}: {error}", file.display()));
                        }
                    }
                }
                Err(error) => errors.push(format!("{}: {error}", file.display())),
            }
        }
        registry.bindings = bindings.to_vec();
        (registry, errors)
    }

    /// Fully qualified names of all user-loaded message types, sorted.
    #[must_use]
    pub fn message_types(&self) -> Vec<String> {
        let mut names: Vec<String> = self.pool.all_messages().map(|message| message.full_name().to_owned()).collect();
        names.sort();
        names
    }

    /// The message type bound to `topic`, if any binding matches.
    #[must_use]
    pub fn binding_for(&self, topic: &str) -> Option<&str> {
        self.bindings
            .iter()
            .find(|binding| crate::topic::matches(&binding.pattern, topic))
            .map(|binding| binding.message_type.as_str())
    }

    /// Decodes `payload` as the message type bound to `topic`.
    ///
    /// # Errors
    /// Fails when no binding matches, the type is unknown or the bytes are not a valid message.
    pub fn decode(&self, topic: &str, payload: &[u8]) -> Result<Value, String> {
        let name =
            self.binding_for(topic).ok_or_else(|| "no Protobuf message type is bound to this topic".to_owned())?;
        let descriptor =
            self.pool.get_message_by_name(name).ok_or_else(|| format!("message type '{name}' is not loaded"))?;
        decode_with(descriptor, payload)
    }

    /// Decodes `payload` as a Sparkplug B payload.
    ///
    /// # Errors
    /// Fails when the bytes are not a valid Sparkplug B message.
    pub fn decode_sparkplug(&self, payload: &[u8]) -> Result<Value, String> {
        let descriptor = self.sparkplug.clone().ok_or_else(|| "the Sparkplug B schema is unavailable".to_owned())?;
        decode_with(descriptor, payload)
    }
}

fn compile(file: &Path) -> Result<DescriptorPool, String> {
    let include = file.parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    let mut compiler = protox::Compiler::new([include]).map_err(|error| error.to_string())?;
    compiler.include_imports(true);
    compiler.open_file(file).map_err(|error| error.to_string())?;
    Ok(compiler.descriptor_pool())
}

fn decode_with(descriptor: MessageDescriptor, payload: &[u8]) -> Result<Value, String> {
    let message = DynamicMessage::decode(descriptor, payload).map_err(|error| error.to_string())?;
    // Keep 64-bit integers numeric so they can be charted; skip nothing so the shape is stable.
    let options = SerializeOptions::new().stringify_64_bit_integers(false).skip_default_fields(true);
    message.serialize_with_options(serde_json::value::Serializer, &options).map_err(|error| error.to_string())
}
