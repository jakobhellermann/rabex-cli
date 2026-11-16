//! The game's managed assemblies (`<game>_Data/Managed/*.dll`): listing them and their types via
//! dotnetdll, and decompiling a type with `ilspycmd`.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, Result, anyhow, bail, ensure};
use dotnetdll::prelude::{ReadOptions, Resolution, TypeIndex};
use dotnetdll::resolved::types::{Kind, TypeDefinition, TypeSource};
use rabex_env::Environment;
use rayon::prelude::*;
use serde::Serialize;

use crate::cli::Format;
use crate::output::{Render, emit, style};

const READ_OPTIONS: ReadOptions = ReadOptions::new()
    .with_skip_method_bodies(true)
    .with_lazy_method_signatures(true)
    .with_lazy_property_signatures(true)
    .with_lazy_attributes(true);

pub fn managed_dir(env: &Environment) -> Result<PathBuf> {
    let data_dir = &env.game_files.game_dir;
    let dir = data_dir.join("Managed");
    ensure!(
        dir.is_dir(),
        "{} has no Managed directory (IL2CPP games have no managed assemblies)",
        data_dir.display()
    );
    Ok(dir)
}

fn assembly_paths(managed: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in
        std::fs::read_dir(managed).with_context(|| format!("reading {}", managed.display()))?
    {
        let path = entry?.path();
        if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("dll"))
        {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

fn stem(path: &Path) -> String {
    path.file_stem().unwrap().to_string_lossy().into_owned()
}

/// The assembly names (without `.dll`) in the game's `Managed` directory.
pub fn assembly_names(env: &Environment) -> Result<Vec<String>> {
    Ok(assembly_paths(&managed_dir(env)?)?
        .iter()
        .map(|path| stem(path))
        .collect())
}

/// The path of the assembly `name`, with or without `.dll`.
pub fn resolve_assembly(env: &Environment, name: &str) -> Result<PathBuf> {
    let managed = managed_dir(env)?;
    let name = name.strip_suffix(".dll").unwrap_or(name);
    let paths = assembly_paths(&managed)?;
    if let Some(path) = paths.iter().find(|path| stem(path) == name) {
        return Ok(path.clone());
    }
    let similar = similar(paths.iter().map(|path| stem(path)), name);
    bail!("no assembly '{name}' in {}{similar}", managed.display())
}

fn parse<'a>(path: &Path, bytes: &'a [u8]) -> Result<Resolution<'a>> {
    Resolution::parse(bytes, READ_OPTIONS).map_err(|e| anyhow!("parsing {}: {e:?}", path.display()))
}

#[derive(Serialize)]
pub struct AssemblyList {
    pub assemblies: Vec<AssemblyEntry>,
}

#[derive(Serialize)]
pub struct AssemblyEntry {
    pub name: String,
    pub version: Option<String>,
    pub size: u64,
}

impl Render for AssemblyList {
    fn render(&self, out: &mut dyn std::io::Write) -> Result<()> {
        let width = self
            .assemblies
            .iter()
            .map(|a| a.name.len())
            .max()
            .unwrap_or(0);
        for assembly in &self.assemblies {
            let version = assembly.version.as_deref().unwrap_or("-");
            writeln!(
                out,
                "{}  {:<15}  {}",
                style::name(&format!("{:<width$}", assembly.name)),
                version,
                style::dim(&format!("{:>7} KiB", assembly.size / 1024))
            )?;
        }
        Ok(())
    }
}

pub fn list_assemblies(env: &Environment) -> Result<AssemblyList> {
    let assemblies = assembly_paths(&managed_dir(env)?)?
        .par_iter()
        .map(|path| -> Result<AssemblyEntry> {
            let bytes =
                std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
            let res = parse(path, &bytes)?;
            Ok(AssemblyEntry {
                name: stem(path),
                version: res.assembly.as_ref().map(|assembly| {
                    let v = assembly.version;
                    format!("{}.{}.{}.{}", v.major, v.minor, v.build, v.revision)
                }),
                size: bytes.len() as u64,
            })
        })
        .collect::<Result<_>>()?;
    Ok(AssemblyList { assemblies })
}

#[derive(Serialize, Clone)]
pub struct TypeEntry {
    /// `Namespace.Outer+Nested`, like `Type.FullName`.
    pub name: String,
    pub kind: &'static str,
}

/// The types an assembly defines, without compiler-generated ones.
pub fn types(path: &Path) -> Result<Vec<TypeEntry>> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let res = parse(path, &bytes)?;
    Ok(res
        .enumerate_type_definitions()
        .filter(|(_, ty)| !ty.name.contains(['<', '>']))
        .map(|(index, ty)| TypeEntry {
            name: full_name(&res, index),
            kind: kind(&res, ty),
        })
        .collect())
}

fn full_name(res: &Resolution, index: TypeIndex) -> String {
    let ty = &res[index];
    match (ty.encloser, ty.namespace.as_deref()) {
        (Some(encloser), _) => format!("{}+{}", full_name(res, encloser), ty.name),
        (None, Some(namespace)) if !namespace.is_empty() => format!("{namespace}.{}", ty.name),
        (None, _) => ty.name.to_string(),
    }
}

fn kind(res: &Resolution, ty: &TypeDefinition) -> &'static str {
    match ty.flags.kind {
        Kind::Interface => "interface",
        Kind::Class => match &ty.extends {
            Some(TypeSource::User(base)) => match base.type_name(res).as_str() {
                "System.Enum" => "enum",
                "System.ValueType" => "struct",
                "System.MulticastDelegate" => "delegate",
                _ => "class",
            },
            _ => "class",
        },
    }
}

#[derive(Serialize)]
pub struct ClassList {
    pub assembly: String,
    pub classes: Vec<TypeEntry>,
}

impl Render for ClassList {
    fn render(&self, out: &mut dyn std::io::Write) -> Result<()> {
        for class in &self.classes {
            writeln!(
                out,
                "{}  {}",
                style::dim(&format!("{:<9}", class.kind)),
                style::class(&class.name)
            )?;
        }
        Ok(())
    }
}

pub fn list_classes(path: &Path, filter: Option<&str>) -> Result<ClassList> {
    let filter = filter.map(str::to_lowercase);
    let classes = types(path)?
        .into_iter()
        .filter(|ty| {
            filter
                .as_ref()
                .is_none_or(|filter| ty.name.to_lowercase().contains(filter))
        })
        .collect();
    Ok(ClassList {
        assembly: stem(path),
        classes,
    })
}

/// The full name of the type `query` names: its full name, or a name it ends with (`Class`,
/// `Outer+Nested`) if that's unique. Nested types may also be written `Outer/Nested`.
pub fn resolve_class(path: &Path, query: &str) -> Result<String> {
    let query = query.replace('/', "+");
    let names: Vec<String> = types(path)?.into_iter().map(|ty| ty.name).collect();
    if names.contains(&query) {
        return Ok(query);
    }
    let matches: Vec<&String> = names
        .iter()
        .filter(|name| name.ends_with(&format!(".{query}")) || name.ends_with(&format!("+{query}")))
        .collect();
    match matches.as_slice() {
        [name] => Ok((*name).clone()),
        [] => {
            let similar = similar(names.iter().cloned(), &query);
            bail!("no class '{query}' in {}{similar}", stem(path))
        }
        _ => bail!(
            "class '{query}' is ambiguous in {}: {}",
            stem(path),
            matches
                .iter()
                .map(|name| name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// ", similar: a, b" for the candidates containing `query` (case-insensitive), or nothing.
fn similar(candidates: impl Iterator<Item = String>, query: &str) -> String {
    let query = query.to_lowercase();
    let similar: Vec<String> = candidates
        .filter(|candidate| candidate.to_lowercase().contains(&query))
        .take(10)
        .collect();
    match similar.is_empty() {
        true => String::new(),
        false => format!("; similar: {}", similar.join(", ")),
    }
}

#[derive(Serialize)]
pub struct Decompiled {
    pub assembly: String,
    pub class: String,
    pub source: String,
}

impl Render for Decompiled {
    fn render(&self, out: &mut dyn std::io::Write) -> Result<()> {
        out.write_all(self.source.as_bytes())?;
        Ok(())
    }
}

/// Decompile the type `class` (a full name) of the assembly at `path`. ilspy renders a nested
/// type as part of its outermost type.
pub fn decompile(managed: &Path, path: &Path, class: &str) -> Result<Decompiled> {
    let outermost = class.split('+').next().unwrap();
    let output = match Command::new("ilspycmd")
        .arg("-r")
        .arg(managed)
        .arg("-t")
        .arg(outermost)
        .arg(path)
        .output()
    {
        Ok(output) => output,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            bail!("ilspycmd not found on PATH (`dotnet tool install --global ilspycmd`)")
        }
        Err(e) => return Err(e).context("running ilspycmd"),
    };
    ensure!(
        output.status.success(),
        "ilspycmd failed for {outermost} in {}: {}",
        path.display(),
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(Decompiled {
        assembly: stem(path),
        class: class.to_owned(),
        source: String::from_utf8(output.stdout).context("ilspycmd output is not utf-8")?,
    })
}

pub fn run_assemblies(env: &Environment, format: Format) -> Result<()> {
    emit(
        &list_assemblies(env)?,
        format,
        &mut std::io::stdout().lock(),
    )
}
