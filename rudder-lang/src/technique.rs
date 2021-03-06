// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2019-2020 Normation SAS

mod from_ir;
mod generate;
mod read;
pub use generate::technique_generate;
pub use read::technique_read;

use crate::{
    cfstrings,
    error::*,
    rudderlang_lib::{LibMethod, RudderlangLib},
};
use colored::Colorize;
use lazy_static::lazy_static;
use regex::{Captures, Regex};
use serde::{
    de::{self, Deserializer, Unexpected},
    Deserialize, Serialize, Serializer,
};
use std::str;

// Techniques are limited subsets of CFEngine in JSON representation
// that only carry method calls and Rudder metadata

// might change later
pub type TechniqueFmt = String;

/// Every Technique substructure has only 1 purpose: represent a Technique as json or rudderlang string
#[derive(Serialize, Deserialize)]
pub struct Technique {
    r#type: String,
    version: u8,
    data: TechniqueData,
}
impl Technique {
    /// creates a Technique that will be used to generate a string representation of a rudderlang or json technique
    pub fn from_json(input: &str, content: &str, is_technique_data: bool) -> Result<Self> {
        info!("|- {} {}", "Parsing".bright_green(), input.bright_yellow());

        if is_technique_data {
            let data = serde_json::from_str::<TechniqueData>(content)
                .map_err(|e| Error::new(format!("Technique from JSON: {}", e)))?;
            Ok(Self {
                r#type: "ncf_techniques".to_owned(),
                version: 2,
                data,
            })
        } else {
            serde_json::from_str::<Self>(content)
                .map_err(|e| Error::new(format!("Technique from JSON: {}", e)))
        }
    }

    pub fn to_json(&self) -> Result<TechniqueFmt> {
        info!(
            "|- {} (translation phase)",
            "Generating JSON code".bright_green()
        );

        serde_json::to_string_pretty(self)
            .map_err(|e| Error::new(format!("Technique to JSON: {}", e)))
    }

    pub fn to_rudderlang(&self, lib: &RudderlangLib) -> Result<TechniqueFmt> {
        info!(
            "|- {} (translation phase)",
            "Generating rudderlang code".bright_green()
        );

        self.data.to_rudderlang(lib)
    }
}

#[derive(Serialize, Deserialize)]
pub struct TechniqueData {
    bundle_name: String,
    description: String,
    name: String,
    version: String,
    #[serde(rename = "parameter")]
    interpolated_parameters: Vec<InterpolatedParameter>,
    #[serde(default = "default_category")] // >=6.2
    category: String,
    method_calls: Vec<MethodCall>,
    #[serde(default)] // >=6.2
    resources: Vec<Resource>,
}
fn default_category() -> String {
    "ncf_techniques".to_owned()
}
impl TechniqueData {
    fn to_rudderlang(&self, lib: &RudderlangLib) -> Result<String> {
        let (parameters_meta, parameter_list): (Vec<String>, Vec<String>) = self
            .interpolated_parameters
            .iter()
            .map(|p| p.to_rudderlang())
            .collect::<Result<Vec<(String, String)>>>()?
            .into_iter()
            .unzip();
        let parameters_meta_fmt = match parameters_meta.is_empty() {
            true => "".to_owned(),
            false => format!("\n  {}\n", parameters_meta.join(",\n  ")),
        };

        let calls = self
            .method_calls
            .iter()
            .map(|c| c.to_rudderlang(lib))
            .collect::<Result<Vec<String>>>()?;
        let calls_fmt = match calls.is_empty() {
            true => "".to_owned(),
            false => format!("\n  {}\n", calls.join("\n\n  ")),
        };

        Ok(format!(
            r#"# Generated from json technique
@format = 0
@name = "{name}"
@description = "{description}"
@version = {version}
@parameters = [{parameters_meta}]

resource {bundle_name}({parameter_list})

{bundle_name} state technique() {{{calls}}}
"#,
            name = self.name,
            description = self.description,
            version = self.version,
            parameters_meta = parameters_meta_fmt,
            bundle_name = self.bundle_name,
            parameter_list = parameter_list.join(", "),
            calls = calls_fmt
        ))
    }
}

#[derive(Serialize, Deserialize, Default)]
pub struct InterpolatedParameter {
    id: String,
    name: String,
    description: String,
}
impl InterpolatedParameter {
    fn to_rudderlang(&self) -> Result<(String, String)> {
        let parameter_meta = format!(
            r#"{{ "name": "{}", "id": "{}", "description": "{}" }}"#,
            self.name, self.id, self.description,
        );
        let parameter = self.name.replace("\"", "").replace(" ", "_").to_owned();

        Ok((parameter_meta, parameter))
    }
}

#[derive(Serialize, Deserialize)]
pub struct MethodCall {
    #[serde(default)]
    parameters: Vec<Value>,
    #[serde(rename = "class_context")]
    condition: String,
    method_name: String,
    component: String,
}
impl MethodCall {
    fn to_rudderlang(&self, lib: &RudderlangLib) -> Result<String> {
        let lib_method: LibMethod = lib.method_from_str(&self.method_name)?;

        let (mut params, template_vars) = self.format_parameters()?;
        // check. note: replace `1` by resource param count? not yet, error if several parameters
        let param_count = 1 + lib_method.state.parameters.len();
        if params.len() != param_count {
            return Err(Error::new(format!(
                "Method {} is expected to have {} parameters, found {}",
                self.method_name,
                param_count,
                params.len()
            )));
        }
        let state_params: Vec<String> = params
            .drain(lib_method.resource.parameters.len()..)
            .collect();

        let mut call = format!(
            "{}({}).{}({})",
            lib_method.resource.name,
            params.join(", "),
            lib_method.state.name,
            state_params.join(", ")
        );
        if self.condition != "any" {
            call = format!("if {} => {}", self.format_condition(&lib)?, call);
        }

        let class_parameter = &self.parameters[lib_method.class_param_index()].value;
        let canonic_parameter = cfstrings::canonify(class_parameter);
        let outcome = format!(" as {}_{}", lib_method.class_prefix(), canonic_parameter);

        Ok(format!(
            "{}@component = \"{}\"\n  {}{}",
            template_vars.join("\n  "),
            &self.component,
            call,
            outcome
        ))
    }

    fn format_parameters(&self) -> Result<(Vec<String>, Vec<String>)> {
        let mut vars = Vec::new();
        let mut template_vars = Vec::new();
        for p in &self.parameters {
            match p.to_rudderlang(template_vars.len())? {
                (var, Some(template_var)) => {
                    vars.push(var);
                    template_vars.push(template_var);
                }
                (var, None) => vars.push(var),
            };
        }
        template_vars.push("".to_owned());
        Ok((vars, template_vars))
    }

    fn format_condition(&self, lib: &RudderlangLib) -> Result<String> {
        lazy_static! {
            static ref CONDITION_RE: Regex = Regex::new(r"([\w${}.]+)").unwrap();
            static ref ANY_RE: Regex = Regex::new(r"(any\.)").unwrap();
        }
        // remove `any.` from condition
        let anyless_condition = ANY_RE.replace_all(&self.condition, "");
        let mut errs = Vec::new();
        // replace all matching words as classes
        let result = CONDITION_RE.replace_all(&anyless_condition, |caps: &Captures| {
            match self.format_method(lib, &caps[1]) {
                Ok(s) => s,
                Err(e) => {
                    errs.push(e);
                    "".into()
                }
            }
        });
        if errs.is_empty() {
            Ok(result.into())
        } else {
            Err(Error::from_vec(errs))
        }
    }

    fn format_method(&self, lib: &RudderlangLib, cond: &str) -> Result<String> {
        // return known system class (formatted as cfengine system)
        if let Some(system) = lib.cf_system(cond) {
            return system;
        }
        // return method if outcome
        if let Some(outcome) = lib.cf_outcome(cond) {
            return Ok(outcome);
        }
        // else
        Err(Error::new(format!(
            "Don't know how to handle class '{}'",
            cond
        )))
    }
}

#[derive(Serialize, Deserialize, Default)]
struct Resource {
    name: String,
    state: Value,
}
impl Resource {
    fn to_rudderlang(&self) -> Result<String> {
        unimplemented!()
        // not sure what it is yet, state = [ new, modified, deleted, untouched ]
        // no idea where / how resources are computed yet
    }
}

#[derive(Serialize, Deserialize, Default)]
struct Value {
    name: String, // not used in rudder-lang
    value: String,
    #[serde(skip_deserializing, rename = "$errors")]
    // only useful when coupled with technique editor
    errors: Vec<String>,
}
impl Value {
    fn new(name: &str, value: &str) -> Self {
        Self {
            name: name.to_owned(),
            value: value.to_owned(),
            errors: Vec::new(),
        }
    }

    fn to_rudderlang(&self, template_len: usize) -> Result<(String, Option<String>)> {
        // rl v2 behavior should make use of this, for now, just a syntax lib
        if cfstrings::parse_string(&self.value).is_err() {
            return Err(Error::new(format!(
                "Invalid variable syntax in '{}'",
                self.value
            )));
        }
        Ok(match self.value.contains('$') {
            true => (
                format!("p{}", template_len),
                Some(format!("let p{} = {:#?}", template_len, self.value)),
            ),
            false => (format!("{:#?}", self.value), None),
        })
    }
}
