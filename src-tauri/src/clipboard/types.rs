pub use quakboard_sync::content::detect_content_type;
use std::collections::HashMap;

/// Detecta el lenguaje de programación usando scoring system
pub fn detect_code_language(text: &str) -> Option<String> {
    let mut scores: HashMap<&str, i32> = HashMap::new();
    let text_lower = text.to_lowercase();
    let lines: Vec<&str> = text.lines().collect();

    // Swift detection
    if text.contains("import SwiftUI")
        || text.contains(
            "import 
  UIKit",
        )
    {
        *scores.entry("swift").or_insert(0) += 15;
    }
    if text.contains("var ") && text.contains(": ") && text.contains("{") {
        *scores.entry("swift").or_insert(0) += 5;
    }
    if text.contains("func ") && text.contains("->") {
        *scores.entry("swift").or_insert(0) += 8;
    }
    if text.contains("@State") || text.contains("@Binding") || text.contains("@ObservedObject") {
        *scores.entry("swift").or_insert(0) += 10;
    }
    if text.contains("struct ") && text.contains(": View") {
        *scores.entry("swift").or_insert(0) += 12;
    }

    // Dart/Flutter detection
    if text.contains("import 'package:flutter/") {
        *scores.entry("dart").or_insert(0) += 15;
    }
    if text.contains("Widget ")
        || text.contains("StatelessWidget")
        || text.contains("StatefulWidget")
    {
        *scores.entry("dart").or_insert(0) += 12;
    }
    if text.contains("@override") && text.contains("Widget build") {
        *scores.entry("dart").or_insert(0) += 10;
    }
    if text.contains("MaterialApp") || text.contains("Scaffold") {
        *scores.entry("dart").or_insert(0) += 8;
    }

    // Docker/Dockerfile detection
    if text.starts_with("FROM ") || lines.iter().any(|l| l.trim().starts_with("FROM ")) {
        *scores.entry("docker").or_insert(0) += 15;
    }
    if text.contains("RUN ") || text.contains("COPY ") || text.contains("ADD ") {
        *scores.entry("docker").or_insert(0) += 8;
    }
    if text.contains("WORKDIR ") || text.contains("EXPOSE ") || text.contains("CMD ") {
        *scores.entry("docker").or_insert(0) += 6;
    }
    if text.contains("ENTRYPOINT") || text.contains("ENV ") {
        *scores.entry("docker").or_insert(0) += 5;
    }

    // Kotlin detection
    if text.contains("package ") && text.contains("import ") && text.contains("fun ") {
        *scores.entry("kotlin").or_insert(0) += 10;
    }
    if text.contains("class ") && text.contains(": ") && text.contains("()") {
        *scores.entry("kotlin").or_insert(0) += 5;
    }
    if text.contains("val ") || text.contains("var ") {
        *scores.entry("kotlin").or_insert(0) += 3;
    }
    if text.contains("fun main") || text.contains("suspend fun") {
        *scores.entry("kotlin").or_insert(0) += 8;
    }

    // C# detection
    if text.contains("using System") || text.contains("using UnityEngine") {
        *scores.entry("csharp").or_insert(0) += 12;
    }
    if text.contains("namespace ") && text.contains("class ") {
        *scores.entry("csharp").or_insert(0) += 10;
    }
    if text.contains("public class") || text.contains("private class") {
        *scores.entry("csharp").or_insert(0) += 8;
    }
    if text.contains("async Task") || text.contains("await ") {
        *scores.entry("csharp").or_insert(0) += 7;
    }

    // TypeScript detection (before JavaScript)
    if text.contains("interface ") || text.contains("type ") && text.contains("=") {
        *scores.entry("typescript").or_insert(0) += 10;
    }
    if text.contains(": string") || text.contains(": number") || text.contains(": boolean") {
        *scores.entry("typescript").or_insert(0) += 8;
    }
    if text.contains("export interface") || text.contains("export type") {
        *scores.entry("typescript").or_insert(0) += 12;
    }
    if text.contains("<T>") || text.contains("<T,") {
        *scores.entry("typescript").or_insert(0) += 5;
    }

    // React/JSX detection
    if text.contains("import React")
        || text.contains("from 'react'")
        || text.contains("from \"react\"")
    {
        *scores.entry("react").or_insert(0) += 12;
    }
    if text.contains("useState") || text.contains("useEffect") || text.contains("useContext") {
        *scores.entry("react").or_insert(0) += 10;
    }
    if text.contains("className=") && text.contains("return (") {
        *scores.entry("react").or_insert(0) += 8;
    }
    if text.contains("export default") && text.contains("=>") {
        *scores.entry("react").or_insert(0) += 5;
    }

    // Svelte detection
    if text.contains("<script")
        && text.contains("</script>")
        && (text.contains("$:") || text.contains("export let") || text.contains("$state"))
    {
        *scores.entry("svelte").or_insert(0) += 15;
    }

    // Vue detection
    if text.contains("<template>") || text.contains("<script setup>") {
        *scores.entry("vue").or_insert(0) += 15;
    }
    if text.contains("defineProps") || text.contains("defineEmits") {
        *scores.entry("vue").or_insert(0) += 10;
    }

    // HTML detection
    if text.contains("<!DOCTYPE") || text.contains("<html") {
        *scores.entry("html").or_insert(0) += 15;
    }
    if text.contains("<head>") || text.contains("<body>") {
        *scores.entry("html").or_insert(0) += 10;
    }

    // Python detection
    if text.starts_with("#!/usr/bin/env python") || text.starts_with("#!/usr/bin/python") {
        *scores.entry("python").or_insert(0) += 15;
    }
    if text.contains("def ") && text.contains(":") {
        *scores.entry("python").or_insert(0) += 10;
    }
    if text.contains("import ") && !text.contains("from \"") && !text.contains("from '") {
        *scores.entry("python").or_insert(0) += 5;
    }
    if text.contains("print(") || text.contains("self.") {
        *scores.entry("python").or_insert(0) += 7;
    }
    if text.contains("if __name__ == \"__main__\":") {
        *scores.entry("python").or_insert(0) += 12;
    }

    // Rust detection
    if text.contains("fn ") && (text.contains("{") || text.contains("->")) {
        *scores.entry("rust").or_insert(0) += 10;
    }
    if text.contains("impl ") || text.contains("struct ") {
        *scores.entry("rust").or_insert(0) += 8;
    }
    if text.contains("pub ") && (text.contains("fn ") || text.contains("struct ")) {
        *scores.entry("rust").or_insert(0) += 7;
    }
    if text.contains("let mut") || text.contains("&mut ") {
        *scores.entry("rust").or_insert(0) += 6;
    }
    if text.contains("use ") && text.contains("::") {
        *scores.entry("rust").or_insert(0) += 5;
    }

    // Go detection
    if text.contains("package main") {
        *scores.entry("go").or_insert(0) += 12;
    }
    if text.contains("func ") && text.contains("{") {
        *scores.entry("go").or_insert(0) += 10;
    }
    if text.contains("import (") || (text.contains("import \"") && text.contains("\"")) {
        *scores.entry("go").or_insert(0) += 8;
    }
    if text.contains("func main()") {
        *scores.entry("go").or_insert(0) += 15;
    }
    if text.contains(":=") {
        *scores.entry("go").or_insert(0) += 5;
    }

    // Java detection
    if text.contains("public class ") || text.contains("private class ") {
        *scores.entry("java").or_insert(0) += 10;
    }
    if text.contains("public static void main") {
        *scores.entry("java").or_insert(0) += 15;
    }
    if text.contains("System.out.println") {
        *scores.entry("java").or_insert(0) += 12;
    }
    if text.contains("@Override") || text.contains("@Autowired") {
        *scores.entry("java").or_insert(0) += 7;
    }

    // C/C++ detection
    if text.contains("#include") {
        *scores.entry("cpp").or_insert(0) += 12;
    }
    if text.contains("int main(") || text.contains("void main(") {
        *scores.entry("cpp").or_insert(0) += 10;
    }
    if text.contains("std::") || text.contains("cout <<") {
        *scores.entry("cpp").or_insert(0) += 8;
    }

    // CSS detection
    if text.contains("{")
        && text.contains("}")
        && text.contains(":")
        && text.contains(";")
        && !text.contains("function")
        && !text.contains("const")
        && !text.contains("let")
    {
        *scores.entry("css").or_insert(0) += 8;
    }
    if text.contains("@media") || text.contains("@keyframes") {
        *scores.entry("css").or_insert(0) += 10;
    }

    // JSON detection
    if ((text.trim().starts_with("{") && text.trim().ends_with("}"))
        || (text.trim().starts_with("[") && text.trim().ends_with("]")))
        && (text.contains("\":") || text.contains("\": "))
    {
        *scores.entry("json").or_insert(0) += 12;
    }

    // SQL detection
    if text_lower.contains("select ")
        && text_lower.contains(
            "from 
  ",
        )
    {
        *scores.entry("sql").or_insert(0) += 12;
    }
    if text_lower.contains("insert into") || text_lower.contains("update ") {
        *scores.entry("sql").or_insert(0) += 10;
    }
    if text_lower.contains("create table") || text_lower.contains("alter table") {
        *scores.entry("sql").or_insert(0) += 10;
    }

    // Markdown detection
    if text.contains("# ") || text.contains("## ") || text.contains("### ") {
        *scores.entry("markdown").or_insert(0) += 8;
    }
    if text.contains("```") {
        *scores.entry("markdown").or_insert(0) += 10;
    }
    if text.contains("[") && text.contains("](") {
        *scores.entry("markdown").or_insert(0) += 6;
    }

    // PHP detection
    if text.contains("<?php") {
        *scores.entry("php").or_insert(0) += 15;
    }
    if text.contains("$_GET") || text.contains("$_POST") || text.contains("$_SERVER") {
        *scores.entry("php").or_insert(0) += 10;
    }

    // Ruby detection
    if text.contains("def ") && text.contains("end") {
        *scores.entry("ruby").or_insert(0) += 10;
    }
    if text.contains("puts ") || text.contains("require ") {
        *scores.entry("ruby").or_insert(0) += 7;
    }

    // Bash/Shell detection
    if text.starts_with("#!/bin/bash") || text.starts_with("#!/bin/sh") {
        *scores.entry("bash").or_insert(0) += 15;
    }
    if text.contains("echo ") || text.contains("export ") {
        *scores.entry("bash").or_insert(0) += 5;
    }

    // JavaScript detection (lowest priority, as it's common)
    if text.contains("function ") || text.contains("const ") || text.contains("let ") {
        *scores.entry("javascript").or_insert(0) += 5;
    }
    if text.contains("console.log") {
        *scores.entry("javascript").or_insert(0) += 7;
    }
    if text.contains("=>") && !scores.contains_key("typescript") {
        *scores.entry("javascript").or_insert(0) += 3;
    }

    // Return language with highest score (minimum 5 points)
    scores
        .into_iter()
        .filter(|(_, score)| *score >= 5)
        .max_by_key(|(_, score)| *score)
        .map(|(lang, _)| lang.to_string())
}

/// Get source app name (placeholder - requires platform-specific implementation)
pub fn get_source_app() -> Option<String> {
    None
}
