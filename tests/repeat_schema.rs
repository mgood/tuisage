use usage::Spec;

#[test]
fn usage_schema_keeps_flag_and_argument_repeat_counts_separate() {
    let spec: Spec = r#"
        name "demo"
        flag "-i --include <pattern>" var=#true var_min=2 var_max=3
        flag "--tags <tag>..."
        flag "--group... <item>..." var=#true var_min=1 var_max=4 {
            arg "<item>..." var=#true var_min=2 var_max=3
        }
        arg "<files>..." var=#true var_min=1 var_max=6
    "#
    .parse()
    .expect("repeatable forms must parse as valid Usage schema");

    let include = spec
        .cmd
        .flags
        .iter()
        .find(|flag| flag.name == "include")
        .unwrap();
    assert!(include.var);
    assert_eq!(include.var_min, Some(2));
    assert_eq!(include.var_max, Some(3));

    let tags = spec
        .cmd
        .flags
        .iter()
        .find(|flag| flag.name == "tags")
        .unwrap();
    assert!(!tags.var);
    assert!(tags.arg.as_ref().unwrap().var);

    let group = spec
        .cmd
        .flags
        .iter()
        .find(|flag| flag.name == "group")
        .unwrap();
    assert!(group.var);
    assert_eq!(group.var_min, Some(1));
    assert_eq!(group.var_max, Some(4));
    let item = group.arg.as_ref().unwrap();
    assert!(item.var);
    assert_eq!(item.var_min, Some(2));
    assert_eq!(item.var_max, Some(3));

    let files = spec
        .cmd
        .args
        .iter()
        .find(|arg| arg.name == "files")
        .unwrap();
    assert!(files.var);
    assert_eq!(files.var_min, Some(1));
    assert_eq!(files.var_max, Some(6));
}
