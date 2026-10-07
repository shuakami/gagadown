use std::path::PathBuf;
use std::{env, fs};

fn main() {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let windows = env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    println!("cargo:rerun-if-changed=../../assets/icon.ico");
    println!("cargo:rerun-if-changed=../../assets/app.manifest");
    if !windows {
        return;
    }
    let assets = root.join("assets").canonicalize().unwrap();
    let p = |n: &str| assets.join(n).display().to_string().replace('\\', "/");
    let ver = env::var("CARGO_PKG_VERSION").unwrap();
    let nums: Vec<&str> = ver.split('.').collect();
    let rc = format!(
        r#"1 ICON "{icon}"
1 24 "{manifest}"
1 VERSIONINFO
FILEVERSION {a},{b},{c},0
PRODUCTVERSION {a},{b},{c},0
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "CompanyName", "GagaDown"
      VALUE "FileDescription", "GagaDown"
      VALUE "ProductName", "GagaDown"
      VALUE "FileVersion", "{ver}"
      VALUE "ProductVersion", "{ver}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
        icon = p("icon.ico"),
        manifest = p("app.manifest"),
        a = nums[0],
        b = nums[1],
        c = nums[2],
    );
    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("res.rc");
    fs::write(&out, rc).unwrap();
    embed_resource::compile(&out, embed_resource::NONE).manifest_optional().unwrap();
}
