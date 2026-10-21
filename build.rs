//! Generate every C struct layout assertion from the public headers.
use std::{env, fs, path::PathBuf};
fn main() {
    let mut generated = String::from("fn append_generated_layout_checks(output: &mut String) {\n");
    let mut structures = 0;
    let mut fields = 0;
    for path in ["include/viem_core.h", "include/viem_startup.h"] {
        println!("cargo:rerun-if-changed={path}");
        let source = fs::read_to_string(path).unwrap();
        let mut clean = String::new();
        let mut rest = source.as_str();
        while let Some(start) = rest.find("/*") {
            clean.push_str(&rest[..start]);
            let end = rest[start + 2..].find("*/").expect("closed C comment") + start + 4;
            rest = &rest[end..];
        }
        clean.push_str(rest);
        for declaration in clean.split("typedef struct ").skip(1) {
            let (name, body) = declaration.split_once('{').expect("named C struct");
            let name = name.trim();
            assert!(name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
            let (body, tail) = body.split_once('}').expect("closed C struct");
            assert_eq!(tail.split(';').next().unwrap().trim(), name);
            structures += 1;
            generated.push_str(&format!("output.push_str(&format!(\"_Static_assert(sizeof({name}) == {{}} , \\\"{name} size\\\");\\n_Static_assert(_Alignof({name}) == {{}}, \\\"{name} alignment\\\");\\n\", std::mem::size_of::<{name}>(), std::mem::align_of::<{name}>()));\n"));
            for field in body.split(';').filter(|field| !field.trim().is_empty()) {
                let field = field
                    .split('[')
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .last()
                    .unwrap()
                    .trim_start_matches('*');
                assert!(
                    field.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
                    "unrecognized field {field}"
                );
                let rust_field = if field == "namespace_id" {
                    "namespace"
                } else {
                    field
                };
                fields += 1;
                generated.push_str(&format!("output.push_str(&format!(\"_Static_assert(offsetof({name}, {field}) == {{}}, \\\"{name}.{field} offset\\\");\\n\", std::mem::offset_of!({name}, {rust_field})));\n"));
            }
        }
    }
    generated.push_str("}\n");
    generated.push_str(&format!(
        "const GENERATED_LAYOUT_COUNTS: (usize, usize) = ({structures}, {fields});\n"
    ));
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("abi_layout_checks.rs"),
        generated,
    )
    .unwrap();
}
