use crate::{
    config::{Config, Repository},
    module::CheckedModule,
};
use aiken_lang::{
    ast::{
        DataType, Definition, Function, ModuleConstant, RecordConstructor, TypeAlias,
        TypedDefinition,
    },
    format,
    tipo::Type,
};
use askama::Template;
use itertools::Itertools;
use pulldown_cmark as markdown;
use serde::Serialize;
use serde_json as json;
use std::{
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, SystemTime},
};

const MAX_COLUMNS: isize = 999;
const VERSION: &str = env!("CARGO_PKG_VERSION");

mod link_tree;

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct DocFile {
    pub path: PathBuf,
    pub content: String,
}

#[derive(Template)]
#[template(path = "module.html")]
struct ModuleTemplate<'a> {
    aiken_version: &'a str,
    breadcrumbs: String,
    page_title: &'a str,
    module_name: String,
    project_name: &'a str,
    project_version: &'a str,
    modules: &'a [DocLink],
    functions: Vec<DocFunction>,
    types: Vec<DocType>,
    constants: Vec<DocConstant>,
    documentation: String,
    source: &'a DocLink,
    timestamp: String,
}

impl<'a> ModuleTemplate<'a> {
    pub fn is_current_module(&self, module: &DocLink) -> bool {
        match module.path.split(".html").next() {
            None => false,
            Some(name) => self.module_name == name,
        }
    }
}

#[derive(Template)]
#[template(path = "page.html")]
struct PageTemplate<'a> {
    aiken_version: &'a str,
    breadcrumbs: &'a str,
    page_title: &'a str,
    project_name: &'a str,
    project_version: &'a str,
    modules: &'a [DocLink],
    content: String,
    source: &'a DocLink,
    timestamp: &'a str,
}

impl<'a> PageTemplate<'a> {
    pub fn is_current_module(&self, _module: &DocLink) -> bool {
        false
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone)]
struct DocLink {
    indent: usize,
    name: String,
    path: String,
}

impl DocLink {
    pub fn is_empty(&self) -> bool {
        self.name.is_empty()
    }

    pub fn is_separator(&self) -> bool {
        self.path.is_empty()
    }
}

/// Generate documentation files for a given project.
///
/// The documentation is built using template files located at the root of this crate.
/// With the documentation, we also build a client-side search index to ease navigation
/// across multiple modules.
pub fn generate_all(root: &Path, config: &Config, modules: Vec<&CheckedModule>) -> Vec<DocFile> {
    let timestamp = new_timestamp();
    let modules_links = generate_modules_links(&modules);

    let source = match &config.repository {
        None => DocLink {
            indent: 0,
            name: String::new(),
            path: String::new(),
        },
        Some(Repository {
            user,
            project,
            platform,
        }) => DocLink {
            indent: 0,
            name: format!("{user}/{project}"),
            path: format!("https://{platform}.com/{user}/{project}"),
        },
    };

    let mut output_files: Vec<DocFile> = vec![];
    let mut search_indexes: Vec<SearchIndex> = vec![];

    for module in &modules {
        let (indexes, file) = generate_module(config, module, &modules_links, &source, &timestamp);
        if !indexes.is_empty() {
            search_indexes.extend(indexes);
            output_files.push(file);
        }
    }

    output_files.extend(generate_static_assets(search_indexes));
    output_files.push(generate_readme(
        root,
        config,
        &modules_links,
        &source,
        &timestamp,
    ));

    output_files
}

fn generate_module(
    config: &Config,
    module: &CheckedModule,
    modules: &[DocLink],
    source: &DocLink,
    timestamp: &Duration,
) -> (Vec<SearchIndex>, DocFile) {
    let mut search_indexes = vec![];

    // Functions
    let functions: Vec<DocFunction> = module
        .ast
        .definitions
        .iter()
        .flat_map(DocFunction::from_definition)
        .collect();
    functions
        .iter()
        .for_each(|function| search_indexes.push(SearchIndex::from_function(module, function)));

    // Types
    let types: Vec<DocType> = module
        .ast
        .definitions
        .iter()
        .flat_map(DocType::from_definition)
        .collect();
    types
        .iter()
        .for_each(|type_info| search_indexes.push(SearchIndex::from_type(module, type_info)));

    // Constants
    let constants: Vec<DocConstant> = module
        .ast
        .definitions
        .iter()
        .flat_map(DocConstant::from_definition)
        .collect();
    constants
        .iter()
        .for_each(|constant| search_indexes.push(SearchIndex::from_constant(module, constant)));

    let is_empty = functions.is_empty() && types.is_empty() && constants.is_empty();

    // Module
    if !is_empty {
        search_indexes.push(SearchIndex::from_module(module));
    }

    let module = ModuleTemplate {
        aiken_version: VERSION,
        breadcrumbs: to_breadcrumbs(&module.name),
        documentation: render_markdown(&module.ast.docs.iter().join("\n")),
        modules,
        project_name: &config.name.repo.to_string(),
        page_title: &format!("{} - {}", module.name, config.name),
        module_name: module.name.clone(),
        project_version: &config.version.to_string(),
        functions,
        types,
        constants,
        source,
        timestamp: timestamp.as_secs().to_string(),
    };

    (
        search_indexes,
        DocFile {
            path: PathBuf::from(format!("{}.html", module.module_name)),
            content: module
                .render()
                .expect("Module documentation template rendering"),
        },
    )
}

fn generate_static_assets(search_indexes: Vec<SearchIndex>) -> Vec<DocFile> {
    let mut assets: Vec<DocFile> = vec![];

    assets.push(DocFile {
        path: PathBuf::from("favicon.svg"),
        content: std::include_str!("../templates/favicon.svg").to_string(),
    });

    assets.push(DocFile {
        path: PathBuf::from("css/atom-one-light.min.css"),
        content: std::include_str!("../templates/css/atom-one-light.min.css").to_string(),
    });

    assets.push(DocFile {
        path: PathBuf::from("css/atom-one-dark.min.css"),
        content: std::include_str!("../templates/css/atom-one-dark.min.css").to_string(),
    });

    assets.push(DocFile {
        path: PathBuf::from("css/index.css"),
        content: std::include_str!("../templates/css/index.css").to_string(),
    });

    assets.push(DocFile {
        path: PathBuf::from("js/highlight.min.js"),
        content: std::include_str!("../templates/js/highlight.min.js").to_string(),
    });

    assets.push(DocFile {
        path: PathBuf::from("js/highlightjs-aiken.js"),
        content: std::include_str!("../templates/js/highlightjs-aiken.js").to_string(),
    });

    assets.push(DocFile {
        path: PathBuf::from("js/lunr.min.js"),
        content: std::include_str!("../templates/js/lunr.min.js").to_string(),
    });

    assets.push(DocFile {
        path: PathBuf::from("js/index.js"),
        content: std::include_str!("../templates/js/index.js").to_string(),
    });

    assets.push(DocFile {
        path: PathBuf::from("search-data.js"),
        content: format!(
            "window.Aiken.initSearch({});",
            json::to_string(&escape_html_contents(search_indexes))
                .expect("search index serialization")
        ),
    });

    assets
}

fn generate_readme(
    root: &Path,
    config: &Config,
    modules: &[DocLink],
    source: &DocLink,
    timestamp: &Duration,
) -> DocFile {
    let path = PathBuf::from("index.html");

    let content = std::fs::read_to_string(root.join("README.md")).unwrap_or_default();

    let template = PageTemplate {
        aiken_version: VERSION,
        breadcrumbs: ".",
        modules,
        project_name: &config.name.repo.to_string(),
        page_title: &config.name.to_string(),
        project_version: &config.version.to_string(),
        content: render_markdown(&content),
        source,
        timestamp: &timestamp.as_secs().to_string(),
    };

    DocFile {
        path,
        content: template.render().expect("Page template rendering"),
    }
}

fn generate_modules_links(modules: &[&CheckedModule]) -> Vec<DocLink> {
    let non_empty_modules = modules
        .iter()
        .filter(|module| {
            module.ast.definitions.iter().any(|def| {
                matches!(
                    def,
                    Definition::Fn(Function { public: true, .. })
                        | Definition::DataType(DataType { public: true, .. })
                        | Definition::TypeAlias(TypeAlias { public: true, .. })
                        | Definition::ModuleConstant(ModuleConstant { public: true, .. })
                )
            })
        })
        .sorted_by(|a, b| a.name.cmp(&b.name))
        .collect_vec();

    let mut links = link_tree::LinkTree::default();

    for module in non_empty_modules {
        links.insert(module.name.as_str());
    }

    links.to_vec()
}

#[derive(Serialize, PartialEq, Eq, PartialOrd, Ord, Clone)]
struct SearchIndex {
    doc: String,
    title: String,
    content: String,
    url: String,
}

impl SearchIndex {
    fn from_function(module: &CheckedModule, function: &DocFunction) -> Self {
        SearchIndex {
            doc: module.name.to_string(),
            title: function.name.to_string(),
            content: format!("{}\n{}", function.signature, function.raw_documentation),
            url: format!("{}.html#{}", module.name, function.name),
        }
    }

    fn from_type(module: &CheckedModule, type_info: &DocType) -> Self {
        let constructors = type_info
            .constructors
            .iter()
            .map(|constructor| {
                format!(
                    "{}\n{}",
                    constructor.definition, constructor.raw_documentation
                )
            })
            .join("\n");

        SearchIndex {
            doc: module.name.to_string(),
            title: type_info.name.to_string(),
            content: format!(
                "{}\n{}\n{}",
                type_info.definition, type_info.raw_documentation, constructors,
            ),
            url: format!("{}.html#{}", module.name, type_info.name),
        }
    }

    fn from_constant(module: &CheckedModule, constant: &DocConstant) -> Self {
        SearchIndex {
            doc: module.name.to_string(),
            title: constant.name.to_string(),
            content: format!("{}\n{}", constant.definition, constant.raw_documentation),
            url: format!("{}.html#{}", module.name, constant.name),
        }
    }

    fn from_module(module: &CheckedModule) -> Self {
        SearchIndex {
            doc: module.name.to_string(),
            title: module.name.to_string(),
            content: module.ast.docs.iter().join("\n"),
            url: format!("{}.html", module.name),
        }
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct DocFunction {
    name: String,
    signature: String,
    documentation: String,
    raw_documentation: String,
    source_url: String,
}

impl DocFunction {
    fn from_definition(def: &TypedDefinition) -> Option<Self> {
        match def {
            Definition::Fn(func_def) if func_def.public => Some(DocFunction {
                name: func_def.name.clone(),
                documentation: func_def
                    .doc
                    .as_deref()
                    .map(render_markdown)
                    .unwrap_or_default(),
                raw_documentation: func_def.doc.as_deref().unwrap_or_default().to_string(),
                signature: format::Formatter::new()
                    .docs_fn_signature(
                        &func_def.name,
                        &func_def.arguments,
                        &func_def.return_annotation,
                        func_def.return_type.clone(),
                    )
                    .to_pretty_string(MAX_COLUMNS),
                source_url: "#todo".to_string(),
            }),
            _ => None,
        }
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct DocConstant {
    name: String,
    definition: String,
    documentation: String,
    raw_documentation: String,
    source_url: String,
}

impl DocConstant {
    fn from_definition(def: &TypedDefinition) -> Option<Self> {
        match def {
            Definition::ModuleConstant(const_def) if const_def.public => Some(DocConstant {
                name: const_def.name.clone(),
                documentation: const_def
                    .doc
                    .as_deref()
                    .map(render_markdown)
                    .unwrap_or_default(),
                raw_documentation: const_def.doc.as_deref().unwrap_or_default().to_string(),
                definition: format::Formatter::new()
                    .docs_const_expr(&const_def.name, &const_def.value)
                    .to_pretty_string(MAX_COLUMNS),
                source_url: "#todo".to_string(),
            }),
            _ => None,
        }
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Debug)]
struct DocType {
    name: String,
    definition: String,
    documentation: String,
    raw_documentation: String,
    constructors: Vec<DocTypeConstructor>,
    parameters: Vec<String>,
    opaque: bool,
    source_url: String,
}

impl DocType {
    fn from_definition(def: &TypedDefinition) -> Option<Self> {
        match def {
            Definition::TypeAlias(info) if info.public => Some(DocType {
                name: info.alias.clone(),
                definition: format::Formatter::new()
                    .docs_type_alias(&info.alias, &info.parameters, &info.annotation)
                    .to_pretty_string(MAX_COLUMNS),
                documentation: info.doc.as_deref().map(render_markdown).unwrap_or_default(),
                raw_documentation: info.doc.as_deref().unwrap_or_default().to_string(),
                constructors: vec![],
                parameters: info.parameters.clone(),
                opaque: false,
                source_url: "#todo".to_string(),
            }),

            Definition::DataType(info) if info.public && !info.opaque => Some(DocType {
                name: info.name.clone(),
                definition: format::Formatter::new()
                    .docs_data_type(
                        &info.name,
                        &info.parameters,
                        &info.constructors,
                        &info.location,
                    )
                    .to_pretty_string(MAX_COLUMNS),
                documentation: info.doc.as_deref().map(render_markdown).unwrap_or_default(),
                raw_documentation: info.doc.as_deref().unwrap_or_default().to_string(),
                constructors: info
                    .constructors
                    .iter()
                    .map(DocTypeConstructor::from_record_constructor)
                    .collect(),
                parameters: info.parameters.clone(),
                opaque: info.opaque,
                source_url: "#todo".to_string(),
            }),

            Definition::DataType(info) if info.public && info.opaque => Some(DocType {
                name: info.name.clone(),
                definition: format::Formatter::new()
                    .docs_opaque_data_type(&info.name, &info.parameters, &info.location)
                    .to_pretty_string(MAX_COLUMNS),
                documentation: info.doc.as_deref().map(render_markdown).unwrap_or_default(),
                raw_documentation: info.doc.as_deref().unwrap_or_default().to_string(),
                constructors: vec![],
                parameters: info.parameters.clone(),
                opaque: info.opaque,
                source_url: "#todo".to_string(),
            }),

            _ => None,
        }
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Debug)]
struct DocTypeConstructor {
    definition: String,
    documentation: String,
    raw_documentation: String,
}

impl DocTypeConstructor {
    fn from_record_constructor(constructor: &RecordConstructor<Rc<Type>>) -> Self {
        let doc_args = constructor
            .arguments
            .iter()
            .filter_map(|arg| match (arg.label.as_deref(), arg.doc.as_deref()) {
                (Some(label), Some(doc)) => Some(format!("#### `.{label}`\n{doc}\n<hr/>\n",)),
                _ => None,
            })
            .join("\n");

        DocTypeConstructor {
            definition: format::Formatter::new()
                .docs_record_constructor(constructor)
                .to_pretty_string(format::MAX_COLUMNS),
            documentation: constructor
                .doc
                .as_deref()
                .map(|doc| render_markdown(&format!("{doc}\n{doc_args}")))
                .unwrap_or_default(),
            raw_documentation: constructor.doc.as_deref().unwrap_or_default().to_string(),
        }
    }
}

// ------ Extra Helpers

fn render_markdown(text: &str) -> String {
    let mut s = String::with_capacity(text.len() * 3 / 2);
    let p = markdown::Parser::new_ext(text, markdown::Options::all());
    markdown::html::push_html(&mut s, p);
    s
}

fn escape_html_contents(indexes: Vec<SearchIndex>) -> Vec<SearchIndex> {
    fn escape_html_content(it: String) -> String {
        it.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('\"', "&quot;")
            .replace('\'', "&#39;")
    }

    indexes
        .into_iter()
        .map(|idx| SearchIndex {
            doc: idx.doc,
            title: idx.title,
            content: escape_html_content(idx.content),
            url: idx.url,
        })
        .collect::<Vec<SearchIndex>>()
}

fn new_timestamp() -> Duration {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("get current timestamp")
}

fn to_breadcrumbs(path: &str) -> String {
    let breadcrumbs = path
        .strip_prefix('/')
        .unwrap_or(path)
        .split('/')
        .skip(1)
        .map(|_| "..")
        .join("/");
    if breadcrumbs.is_empty() {
        ".".to_string()
    } else {
        breadcrumbs
    }
}

#[test]
fn to_breadcrumbs_test() {
    // Pages
    assert_eq!(to_breadcrumbs("a.html"), ".");
    assert_eq!(to_breadcrumbs("/a.html"), ".");
    assert_eq!(to_breadcrumbs("/a/b.html"), "..");
    assert_eq!(to_breadcrumbs("/a/b/c.html"), "../..");

    // Modules
    assert_eq!(to_breadcrumbs("a"), ".");
    assert_eq!(to_breadcrumbs("a/b"), "..");
    assert_eq!(to_breadcrumbs("a/b/c"), "../..");
}
