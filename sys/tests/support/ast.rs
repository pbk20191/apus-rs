//! The declarations of include/apus/abi.h as libclang sees them: name, link name, marker, calling convention and
//! signature. Shared by the lowering and fresh tests.
#![allow(dead_code, non_upper_case_globals)]

use clang_sys::*;
use std::ffi::{c_void, CStr, CString};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Function,
    /// A typedef of a function or function pointer type.
    FunctionType,
    Data,
}

#[derive(Clone, Debug)]
pub struct Decl {
    pub name: String,
    /// The symbol it links to: its asm label without the leading `_`, else its name.
    pub link_name: String,
    pub kind: Kind,
    /// `direct`, `c`, `thunk`, `async`, `internal`; empty when the declaration has no `apus:` marker.
    pub marker: String,
    /// `15.0` for `APUS_AVAILABLE(15.0)`.
    pub available: Option<String>,
    pub swift_cc: bool,
    pub ret: String,
    pub params: Vec<String>,
}

pub fn include_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("include")
}

pub fn header() -> PathBuf {
    include_dir().join("apus/abi.h")
}

fn text(s: CXString) -> String {
    unsafe {
        let out = CStr::from_ptr(clang_getCString(s)).to_string_lossy().into_owned();
        clang_disposeString(s);
        out
    }
}

fn spelling(t: CXType) -> String {
    text(unsafe { clang_getTypeSpelling(t) })
}

struct Walk {
    decls: Vec<Decl>,
}

extern "C" fn visit_attrs(cursor: CXCursor, _parent: CXCursor, data: CXClientData) -> CXChildVisitResult {
    let decl = unsafe { &mut *(data as *mut Decl) };
    let kind = unsafe { clang_getCursorKind(cursor) };
    let value = text(unsafe { clang_getCursorSpelling(cursor) });
    if kind == CXCursor_AnnotateAttr {
        if let Some(version) = value.strip_prefix("apus:available:") {
            decl.available = Some(version.to_string());
        } else if let Some(marker) = value.strip_prefix("apus:") {
            decl.marker = marker.to_string();
        }
    } else if kind == CXCursor_AsmLabelAttr {
        decl.link_name = value.strip_prefix('_').unwrap_or(&value).to_string();
    }
    CXChildVisit_Continue
}

fn function_type(t: CXType) -> Option<CXType> {
    let t = unsafe { clang_getCanonicalType(t) };
    match t.kind {
        CXType_FunctionProto | CXType_FunctionNoProto => Some(t),
        CXType_Pointer => function_type(unsafe { clang_getPointeeType(t) }),
        _ => None,
    }
}

extern "C" fn visit(cursor: CXCursor, _parent: CXCursor, data: CXClientData) -> CXChildVisitResult {
    let walk = unsafe { &mut *(data as *mut Walk) };
    unsafe {
        let mut file: CXFile = std::ptr::null_mut();
        clang_getFileLocation(clang_getCursorLocation(cursor), &mut file, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut());
        if file.is_null() || !text(clang_getFileName(file)).contains("/include/apus/") {
            return CXChildVisit_Continue;
        }
    }
    let cursor_kind = unsafe { clang_getCursorKind(cursor) };
    let (kind, signature) = match cursor_kind {
        CXCursor_FunctionDecl => (Kind::Function, function_type(unsafe { clang_getCursorType(cursor) })),
        CXCursor_TypedefDecl => match function_type(unsafe { clang_getTypedefDeclUnderlyingType(cursor) }) {
            Some(f) => (Kind::FunctionType, Some(f)),
            None => return CXChildVisit_Continue,
        },
        CXCursor_VarDecl => (Kind::Data, None),
        _ => return CXChildVisit_Continue,
    };
    let name = text(unsafe { clang_getCursorSpelling(cursor) });
    let mut decl = Decl { link_name: name.clone(), name, kind, marker: String::new(), available: None, swift_cc: false, ret: String::new(), params: Vec::new() };
    if let Some(f) = signature {
        let cc = unsafe { clang_getFunctionTypeCallingConv(f) };
        decl.swift_cc = cc == CXCallingConv_Swift || cc == CXCallingConv_SwiftAsync;
        decl.ret = spelling(unsafe { clang_getResultType(f) });
        let count = unsafe { clang_getNumArgTypes(f) }.max(0) as u32;
        decl.params = (0..count).map(|i| spelling(unsafe { clang_getArgType(f, i) })).collect();
    }
    unsafe { clang_visitChildren(cursor, visit_attrs, &mut decl as *mut Decl as *mut c_void) };
    walk.decls.push(decl);
    CXChildVisit_Continue
}

/// Every function, function typedef and variable declared under include/apus, in header order. `defines` are
/// extra `-D` options (for example `APUS_BINDGEN`).
pub fn declarations(defines: &[&str]) -> Vec<Decl> {
    clang_sys::load().ok();
    // Parsed as apus's own C sees it (APUS_SYS_BUILD: the boundary identifiers carry their namespace).
    let mut args: Vec<CString> =
        vec![CString::new("-xc").unwrap(), CString::new("-DAPUS_SYS_BUILD").unwrap(), CString::new(format!("-I{}", include_dir().display())).unwrap()];
    args.extend(defines.iter().map(|d| CString::new(format!("-D{d}")).unwrap()));
    let argv: Vec<*const i8> = args.iter().map(|a| a.as_ptr()).collect();
    let path = CString::new(header().to_str().unwrap()).unwrap();
    unsafe {
        let index = clang_createIndex(0, 1);
        let unit = clang_parseTranslationUnit(index, path.as_ptr(), argv.as_ptr(), argv.len() as i32, std::ptr::null_mut(), 0, CXTranslationUnit_None);
        assert!(!unit.is_null(), "libclang could not parse {}", header().display());
        let diagnostics: Vec<String> = (0..clang_getNumDiagnostics(unit))
            .map(|i| clang_getDiagnostic(unit, i))
            .filter(|d| clang_getDiagnosticSeverity(*d) >= CXDiagnostic_Error)
            .map(|d| text(clang_formatDiagnostic(d, clang_defaultDiagnosticDisplayOptions())))
            .collect();
        assert!(diagnostics.is_empty(), "{} does not compile:\n{}", header().display(), diagnostics.join("\n"));
        let mut walk = Walk { decls: Vec::new() };
        clang_visitChildren(clang_getTranslationUnitCursor(unit), visit, &mut walk as *mut Walk as *mut c_void);
        clang_disposeTranslationUnit(unit);
        clang_disposeIndex(index);
        walk.decls
    }
}

/// The clang version libclang reports.
pub fn clang_version() -> String {
    clang_sys::load().ok();
    text(unsafe { clang_getClangVersion() })
}
