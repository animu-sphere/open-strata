//! Real-CMake regression tests for the copied plugin helper
//! (`templates/_shared/cmake/OpenStrataPlugin.cmake`).
//!
//! They build a pxr-free shared library through `openstrata_configure_plugin`
//! and `openstrata_install_plugin_bundle`, then check that every generated
//! `plugInfo.json` names the file the target actually produced, in the build
//! tree's in-place layout and in each configuration's install tree. Target
//! naming overrides (`OUTPUT_NAME`, `PREFIX`, `SUFFIX`, `DEBUG_POSTFIX`) are
//! applied *after* the helper runs, as a bundle author would.
//!
//! Needs `cmake` and a C++ compiler; skipped when `cmake` is not on PATH or
//! `OST_SKIP_CMAKE_HELPER_TESTS` is set. The platform's default generator is
//! used, so Linux CI exercises a single-config generator and Windows a
//! multi-config (Visual Studio) one.

use std::path::{Path, PathBuf};
use std::process::Command;

const HELPER: &str = include_str!("../../../templates/_shared/cmake/OpenStrataPlugin.cmake");

const PLUG_INFO_IN: &str = r#"{
    "Plugins": [
        {
            "Type": "library",
            "Name": "MiniPlugin",
            "Root": ".",
            "LibraryPath": "@OPENSTRATA_PLUGIN_LIBRARY_PATH@",
            "ResourcePath": ".",
            "Info": {}
        }
    ]
}
"#;

const SOURCE: &str = r#"#ifdef _WIN32
__declspec(dllexport)
#endif
int openstrata_mini() { return 42; }
"#;

const CMAKE_LISTS: &str = r#"cmake_minimum_required(VERSION 3.22)
project(Mini LANGUAGES CXX)
include("${CMAKE_CURRENT_SOURCE_DIR}/cmake/OpenStrataPlugin.cmake")
add_library(MiniPlugin SHARED src/mini.cpp)
openstrata_configure_plugin(
    TARGET MiniPlugin
    PLUG_INFO_INPUT "${CMAKE_CURRENT_SOURCE_DIR}/plugin/resources/mini/plugInfo.json.in"
    PLUG_INFO_OUTPUT "${CMAKE_CURRENT_SOURCE_DIR}/plugin/resources/mini/plugInfo.json")
if(MINI_OUTPUT_NAME)
    set_target_properties(MiniPlugin PROPERTIES OUTPUT_NAME "${MINI_OUTPUT_NAME}")
endif()
if(DEFINED MINI_PREFIX)
    set_target_properties(MiniPlugin PROPERTIES PREFIX "${MINI_PREFIX}")
endif()
if(MINI_SUFFIX)
    set_target_properties(MiniPlugin PROPERTIES SUFFIX "${MINI_SUFFIX}")
endif()
if(MINI_DEBUG_POSTFIX)
    set_target_properties(MiniPlugin PROPERTIES DEBUG_POSTFIX "${MINI_DEBUG_POSTFIX}")
endif()
openstrata_install_plugin_bundle(
    TARGET MiniPlugin
    RESOURCES "${CMAKE_CURRENT_SOURCE_DIR}/plugin/resources/mini/plugInfo.json"
    RESOURCE_DESTINATION "@RESOURCE_DESTINATION@")
"#;

const PLUG_INFO: &str = "plugin/resources/mini/plugInfo.json";

fn cmake_available() -> bool {
    if std::env::var_os("OST_SKIP_CMAKE_HELPER_TESTS").is_some() {
        return false;
    }
    Command::new("cmake").arg("--version").output().is_ok()
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "ost-cmake-helper-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn mini_bundle(root: &Path, resource_destination: &str) {
    write(&root.join("cmake/OpenStrataPlugin.cmake"), HELPER);
    write(&root.join("src/mini.cpp"), SOURCE);
    write(
        &root.join("plugin/resources/mini/plugInfo.json.in"),
        PLUG_INFO_IN,
    );
    write(
        &root.join("CMakeLists.txt"),
        &CMAKE_LISTS.replace("@RESOURCE_DESTINATION@", resource_destination),
    );
}

fn cmake(args: &[&str]) -> std::process::Output {
    Command::new("cmake").args(args).output().unwrap()
}

fn ok(output: std::process::Output, what: &str) {
    assert!(
        output.status.success(),
        "{what} failed\n--- stdout\n{}\n--- stderr\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn path_str(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// The `LibraryPath` in `<tree>/plugin/resources/mini/plugInfo.json`, after
/// asserting it carries no unevaluated token and resolves to a real file.
fn resolved_library(tree: &Path) -> String {
    let plug_info = tree.join(PLUG_INFO);
    let text = std::fs::read_to_string(&plug_info)
        .unwrap_or_else(|e| panic!("{}: {e}", plug_info.display()));
    for token in ["@", "{{", "$<"] {
        assert!(
            !text.contains(token),
            "{} still contains `{token}`:\n{text}",
            plug_info.display()
        );
    }
    let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
    let library_path = doc["Plugins"][0]["LibraryPath"]
        .as_str()
        .unwrap()
        .to_string();
    let library = plug_info.parent().unwrap().join(&library_path);
    assert!(
        library.is_file(),
        "{}: LibraryPath '{library_path}' does not resolve to a built library",
        plug_info.display()
    );
    library_path
}

/// Configure, build and install one configuration into `<root>/install-<config>`.
fn build_config(root: &Path, config: &str, extra: &[&str]) -> PathBuf {
    let source = path_str(root);
    let build = path_str(&root.join(format!("build-{config}")));
    let prefix = root.join(format!("install-{config}"));
    let build_type = format!("-DCMAKE_BUILD_TYPE={config}");
    let mut configure = vec!["-S", &source, "-B", &build, &build_type];
    configure.extend_from_slice(extra);
    ok(cmake(&configure), "configure");
    ok(cmake(&["--build", &build, "--config", config]), "build");
    ok(
        cmake(&[
            "--install",
            &build,
            "--config",
            config,
            "--prefix",
            &path_str(&prefix),
        ]),
        "install",
    );
    prefix
}

fn file_name(library_path: &str) -> &str {
    library_path.rsplit('/').next().unwrap()
}

#[test]
fn library_path_follows_the_target_filename_through_build_and_install() {
    if !cmake_available() {
        eprintln!("skipping: cmake not on PATH (or OST_SKIP_CMAKE_HELPER_TESTS set)");
        return;
    }
    let suffix = std::env::consts::DLL_SUFFIX;
    let cases: [(&str, &[&str], &str); 4] = [
        ("default", &[], "libMiniPlugin"),
        ("output-name", &["-DMINI_OUTPUT_NAME=renamed"], "librenamed"),
        ("no-prefix", &["-DMINI_PREFIX="], "MiniPlugin"),
        (
            "suffix",
            &["-DMINI_SUFFIX=.openstrata"],
            "libMiniPlugin.openstrata",
        ),
    ];
    for (name, extra, stem) in cases {
        let root = scratch(name);
        mini_bundle(&root, "plugin/resources/mini");
        let installed = build_config(&root, "Release", extra);
        let expected = if name == "suffix" {
            stem.to_string()
        } else {
            format!("{stem}{suffix}")
        };
        // In-place layout: source-tree plugInfo.json + bundle lib/.
        let in_place = resolved_library(&root);
        assert_eq!(file_name(&in_place), expected, "{name}: in-place");
        assert!(in_place.starts_with("../../../lib/"), "{name}: {in_place}");
        // Install tree, loadable without the source tree.
        assert_eq!(
            resolved_library(&installed),
            in_place,
            "{name}: installed plugInfo differs from in-place"
        );
        std::fs::remove_dir_all(&root).ok();
    }
}

#[test]
fn per_config_postfix_reaches_each_installed_plug_info() {
    if !cmake_available() {
        eprintln!("skipping: cmake not on PATH (or OST_SKIP_CMAKE_HELPER_TESTS set)");
        return;
    }
    let suffix = std::env::consts::DLL_SUFFIX;
    let root = scratch("postfix");
    mini_bundle(&root, "plugin/resources/mini");
    let postfix = ["-DMINI_DEBUG_POSTFIX=_d"];
    let release = build_config(&root, "Release", &postfix);
    let debug = build_config(&root, "Debug", &postfix);
    assert_eq!(
        file_name(&resolved_library(&release)),
        format!("libMiniPlugin{suffix}")
    );
    assert_eq!(
        file_name(&resolved_library(&debug)),
        format!("libMiniPlugin_d{suffix}")
    );
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn install_destination_at_another_depth_fails_configure() {
    if !cmake_available() {
        eprintln!("skipping: cmake not on PATH (or OST_SKIP_CMAKE_HELPER_TESTS set)");
        return;
    }
    let root = scratch("depth");
    mini_bundle(&root, "plugin/mini");
    let output = cmake(&["-S", &path_str(&root), "-B", &path_str(&root.join("build"))]);
    assert!(!output.status.success(), "configure should fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Keep the RESOURCE_DESTINATION at the same depth"),
        "{stderr}"
    );
    std::fs::remove_dir_all(&root).ok();
}
