use usage::{Spec, SpecArg, SpecCommand, SpecFlag};

#[derive(Clone)]
pub enum FieldKind {
    Flag(Box<SpecFlag>),
    Arg(Box<SpecArg>),
}
#[derive(Clone)]
pub struct Field {
    pub id: String,
    pub path: Vec<String>,
    pub name: String,
    pub kind: FieldKind,
}
pub fn field_id(path: &[String], kind: &str, name: &str, global: bool) -> String {
    let prefix = if global {
        "global".into()
    } else if path.is_empty() {
        "root".into()
    } else {
        format!(
            "commands/{}",
            path.iter().map(|s| escape(s)).collect::<Vec<_>>().join("/")
        )
    };
    format!("{prefix}/{kind}/{}", escape(name))
}
fn escape(s: &str) -> String {
    s.replace('~', "~0").replace('/', "~1")
}

pub fn fields(spec: &Spec) -> Vec<Field> {
    fn collect(cmd: &SpecCommand, path: &[String], result: &mut Vec<Field>) {
        for flag in &cmd.flags {
            result.push(Field {
                id: field_id(path, "flags", &flag.name, path.is_empty() && flag.global),
                path: path.to_vec(),
                name: flag.name.clone(),
                kind: FieldKind::Flag(Box::new(flag.clone())),
            });
        }
        for arg in &cmd.args {
            result.push(Field {
                id: field_id(path, "args", &arg.name, false),
                path: path.to_vec(),
                name: arg.name.clone(),
                kind: FieldKind::Arg(Box::new(arg.clone())),
            });
        }
        for (name, sub) in &cmd.subcommands {
            let mut subpath = path.to_vec();
            subpath.push(name.clone());
            collect(sub, &subpath, result);
        }
    }
    let mut result = Vec::new();
    collect(&spec.cmd, &[], &mut result);
    result
}
