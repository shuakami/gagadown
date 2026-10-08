use fluent_syntax::{ast::*, parser};
use std::collections::{BTreeMap, BTreeSet};

type Variables = BTreeSet<String>;

fn arguments(args: &CallArguments<&str>, vars: &mut Variables) {
    for arg in &args.positional { inline(arg, vars); }
    for arg in &args.named { inline(&arg.value, vars); }
}

fn inline(value: &InlineExpression<&str>, vars: &mut Variables) {
    match value {
        InlineExpression::VariableReference { id } => { vars.insert(id.name.to_owned()); }
        InlineExpression::Placeable { expression: expr } => expression(expr, vars),
        InlineExpression::FunctionReference { arguments: args, .. } => arguments(args, vars),
        InlineExpression::TermReference { arguments: Some(args), .. } => arguments(args, vars),
        InlineExpression::MessageReference { .. } | InlineExpression::TermReference { .. } => {
            panic!("Catalog references require transitive argument validation before use");
        }
        InlineExpression::StringLiteral { .. } | InlineExpression::NumberLiteral { .. } => {}
    }
}

fn expression(expr: &Expression<&str>, vars: &mut Variables) {
    match expr {
        Expression::Inline(value) => inline(value, vars),
        Expression::Select { selector, variants } => {
            inline(selector, vars);
            for variant in variants { pattern(&variant.value, vars); }
        }
    }
}

fn pattern(value: &Pattern<&str>, vars: &mut Variables) {
    for element in &value.elements {
        if let PatternElement::Placeable { expression: expr } = element { expression(expr, vars); }
    }
}

fn schema(source: &str) -> BTreeMap<String, Variables> {
    let resource = parser::parse(source).expect("Valid FTL syntax");
    let mut result = BTreeMap::new();
    for entry in resource.body {
        match entry {
            Entry::Message(message) => {
                let mut vars = Variables::new();
                pattern(message.value.as_ref().expect("Every message has a value"), &mut vars);
                assert!(result.insert(message.id.name.to_owned(), vars).is_none(), "Duplicate key");
                for attribute in message.attributes {
                    let mut vars = Variables::new();
                    pattern(&attribute.value, &mut vars);
                    assert!(result.insert(format!("{}.{}", message.id.name, attribute.id.name), vars).is_none());
                }
            }
            Entry::Comment(_) | Entry::GroupComment(_) | Entry::ResourceComment(_) => {}
            _ => panic!("Unsupported catalog entry must be validated before use"),
        }
    }
    result
}

#[test]
fn bilingual_keys_and_all_nested_arguments_match() {
    assert_eq!(schema(include_str!("../locales/en.ftl")), schema(include_str!("../locales/zh-CN.ftl")));
}

#[test]
fn selector_and_nested_argument_changes_are_detected() {
    let a = schema("items = { $count ->\n [one] { $name }\n *[other] Many\n}\n");
    let b = schema("items = { $count ->\n [one] { $title }\n *[other] Many\n}\n");
    assert_ne!(a, b);
    assert_eq!(a["items"], BTreeSet::from(["count".to_owned(), "name".to_owned()]));
}
