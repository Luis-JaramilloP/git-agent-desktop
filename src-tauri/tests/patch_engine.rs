//! Pruebas del motor determinista: parcheo atomico (Seccion 7 / 12.1),
//! normalizacion CRLF, seguridad de rutas, arbol filtrado y esquema JSON.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use git_agent_desktop_lib::agent::slugify;
use git_agent_desktop_lib::ai::client::{extract_json, parse_plan};
use git_agent_desktop_lib::commands::patch::{
    apply, normalize_newlines, preview, restore, sanitize_rel_path, unified_diff, PatchBlock,
};
use git_agent_desktop_lib::tree;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

struct TempRepo {
    path: PathBuf,
}

impl TempRepo {
    fn new(tag: &str) -> Self {
        let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "gad-test-{}-{}-{}",
            tag,
            std::process::id(),
            unique
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("crear repositorio temporal");
        Self { path }
    }

    fn write(&self, relative: &str, content: &[u8]) -> PathBuf {
        let full = self.path.join(relative);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).expect("crear directorio");
        }
        fs::write(&full, content).expect("escribir archivo");
        full
    }

    fn read(&self, relative: &str) -> String {
        fs::read_to_string(self.path.join(relative)).expect("leer archivo")
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn block(path: &str, search: &str, replace: &str) -> PatchBlock {
    PatchBlock {
        file_path: path.to_string(),
        search_content: search.to_string(),
        replace_content: replace.to_string(),
    }
}

fn find_last(haystack: &[u8], needle: u8) -> Option<usize> {
    haystack.iter().rposition(|byte| *byte == needle)
}

#[test]
fn aplica_bloque_y_preserva_crlf() {
    let repo = TempRepo::new("crlf");
    repo.write(
        "src/main.c",
        b"int main(void) {\r\n    return 0;\r\n}\r\n",
    );

    // El modelo responde siempre con LF.
    let blocks = vec![block("src/main.c", "return 0;", "return 42;")];
    let (changes, snapshots, warnings) =
        apply(&repo.path, &blocks).expect("el parche debe aplicarse");

    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].additions, 1);
    assert_eq!(changes[0].deletions, 1);
    assert!(warnings.is_empty());
    assert_eq!(snapshots.len(), 1);

    let raw = fs::read(repo.path.join("src/main.c")).expect("leer bytes");
    let text = String::from_utf8(raw.clone()).expect("utf8");

    assert!(text.contains("return 42;"), "debe reemplazar el contenido");
    assert!(
        !text.replace("\r\n", "").contains('\n'),
        "no debe quedar ningun LF suelto: los saltos deben seguir siendo CRLF"
    );
    assert_eq!(
        raw.iter().filter(|byte| **byte == b'\n').count(),
        3,
        "el archivo conserva sus 3 lineas"
    );
}

#[test]
fn aplica_bloque_sobre_archivo_lf_sin_convertirlo() {
    let repo = TempRepo::new("lf");
    repo.write("src/lib.rs", b"fn a() {}\nfn b() {}\n");

    apply(&repo.path, &[block("src/lib.rs", "fn b() {}", "fn b() { todo!() }")])
        .expect("parche aplicado");

    let text = repo.read("src/lib.rs");
    assert!(text.contains("fn b() { todo!() }"));
    assert!(!text.contains('\r'), "un archivo LF no debe ganar CRLF");
}

#[test]
fn search_inexistente_no_modifica_el_archivo() {
    let repo = TempRepo::new("nohit");
    let original = "alpha\nbeta\n";
    repo.write("app.txt", original.as_bytes());

    let error = apply(&repo.path, &[block("app.txt", "gamma", "delta")])
        .expect_err("debe fallar si el SEARCH no coincide");

    assert!(error.contains("no coincide"), "mensaje claro: {}", error);
    assert_eq!(repo.read("app.txt"), original);
}

#[test]
fn atomicidad_entre_archivos() {
    let repo = TempRepo::new("atomic");
    repo.write("uno.txt", b"contenido uno\n");
    repo.write("dos.txt", b"contenido dos\n");

    // El segundo bloque no encaja: no debe escribirse ninguno.
    let blocks = vec![
        block("uno.txt", "contenido uno", "contenido UNO"),
        block("dos.txt", "texto inexistente", "x"),
    ];

    assert!(apply(&repo.path, &blocks).is_err());
    assert_eq!(repo.read("uno.txt"), "contenido uno\n");
    assert_eq!(repo.read("dos.txt"), "contenido dos\n");
}

#[test]
fn crea_archivo_nuevo_con_search_vacio() {
    let repo = TempRepo::new("crear");
    let blocks = vec![block("docs/nuevo.md", "", "# Documento\n\nContenido.\n")];

    let (changes, _snapshots, _warnings) = apply(&repo.path, &blocks).expect("crear archivo");
    assert!(changes[0].created, "debe marcarse como archivo creado");
    assert_eq!(repo.read("docs/nuevo.md"), "# Documento\n\nContenido.\n");
}

#[test]
fn bloquea_rutas_fuera_del_repositorio() {
    let repo = TempRepo::new("traversal");
    repo.write("ok.txt", b"x\n");

    for malicious in ["../evil.txt", "..\\evil.txt", "C:\\Windows\\System32\\evil.dll", "/etc/passwd"] {
        assert!(
            sanitize_rel_path(&repo.path, malicious).is_err(),
            "la ruta '{}' debe rechazarse",
            malicious
        );
    }

    assert!(sanitize_rel_path(&repo.path, "src/../ok.txt").is_err(), ".. interno debe rechazarse");
    assert!(sanitize_rel_path(&repo.path, ".git/config").is_err(), ".git debe rechazarse");
    assert!(sanitize_rel_path(&repo.path, "src/ok.txt").is_ok());
}

#[test]
fn restaura_los_archivos_originales() {
    let repo = TempRepo::new("restore");
    let original = "linea uno\nlinea dos\n";
    repo.write("nota.txt", original.as_bytes());

    let (changes, snapshots, _) = apply(
        &repo.path,
        &[
            block("nota.txt", "linea dos", "linea DOS"),
            block("nuevo.txt", "", "archivo creado\n"),
        ],
    )
    .expect("aplicar");

    assert_eq!(changes.len(), 2);
    assert!(repo.path.join("nuevo.txt").exists());

    let restored = restore(&repo.path, &snapshots).expect("restaurar");
    assert_eq!(restored.len(), 2);
    assert_eq!(repo.read("nota.txt"), original);
    assert!(!repo.path.join("nuevo.txt").exists(), "el archivo creado se elimina");
}

#[test]
fn preview_no_escribe_en_disco() {
    let repo = TempRepo::new("preview");
    repo.write("a.txt", b"uno\ndos\n");

    let changes = preview(&repo.path, &[block("a.txt", "dos", "DOS")]).expect("preview");
    assert_eq!(changes.len(), 1);
    assert_eq!(repo.read("a.txt"), "uno\ndos\n", "preview no debe tocar el disco");
    assert!(changes[0].diff.contains("--- a/a.txt"));
    assert!(changes[0].diff.contains("+++ b/a.txt"));
    assert!(changes[0].diff.contains("-dos"));
    assert!(changes[0].diff.contains("+DOS"));
}

#[test]
fn diff_unificado_cuenta_lineas() {
    let (diff, additions, deletions) = unified_diff("x.rs", "a\nb\nc\n", "a\nB\nc\n");
    assert_eq!(additions, 1);
    assert_eq!(deletions, 1);
    assert!(diff.contains("@@"));
    assert!(diff.contains("-b"));
    assert!(diff.contains("+B"));
}

#[test]
fn normaliza_saltos_de_linea() {
    assert_eq!(normalize_newlines("a\r\nb\r\n"), "a\nb\n");
    assert_eq!(normalize_newlines("a\nb\n"), "a\nb\n");
}

#[test]
fn avisa_cuando_el_search_es_ambiguo_pero_aplica_el_primero() {
    let repo = TempRepo::new("ambiguo");
    repo.write("dup.txt", b"valor\nvalor\n");

    let (_changes, _snapshots, warnings) =
        apply(&repo.path, &[block("dup.txt", "valor", "otro")]).expect("aplicar");
    assert!(!warnings.is_empty(), "debe avisar de la ambiguedad");
}

#[test]
fn dos_bloques_sobre_el_mismo_archivo() {
    let repo = TempRepo::new("multi");
    repo.write("m.txt", b"uno\ndos\ntres\n");

    apply(
        &repo.path,
        &[block("m.txt", "uno", "1"), block("m.txt", "tres", "3")],
    )
    .expect("aplicar dos bloques");

    assert_eq!(repo.read("m.txt"), "1\ndos\n3\n");
}

#[test]
fn el_arbol_filtra_ruido_y_secretos() {
    let repo = TempRepo::new("tree");
    repo.write("src/app.ts", b"export const x = 1;\n");
    repo.write("README.md", b"# Doc\n");
    repo.write("node_modules/pkg/index.js", b"module.exports = 1;\n");
    repo.write("target/debug/basura.rs", b"fn nada() {}\n");
    repo.write("package-lock.json", b"{}\n");
    repo.write("logo.png", &[0x89, 0x50, 0x4e, 0x47, 0, 0, 0, 0]);
    repo.write(".env", b"SECRET=abc\n");
    repo.write("certs/server.pem", b"-----BEGIN-----\n");

    let built = tree::build(&repo.path, 100);
    let files = built.files.clone();

    assert!(files.contains(&"src/app.ts".to_string()));
    assert!(files.contains(&"README.md".to_string()));
    assert!(!files.iter().any(|file| file.contains("node_modules")));
    assert!(!files.iter().any(|file| file.starts_with("target/")));
    assert!(!files.iter().any(|file| file.contains("package-lock")));
    assert!(!files.iter().any(|file| file.ends_with(".png")));
    assert!(!files.iter().any(|file| file == ".env"), ".env nunca debe enviarse al modelo");
    assert!(built.skipped_secrets.iter().any(|file| file == ".env"));
    assert!(built.skipped_secrets.iter().any(|file| file == "certs/server.pem"));
    assert!(!built.is_git_repo, "sin .git debe usar el recorrido de respaldo");
}

#[test]
fn lee_archivos_del_repositorio_normalizando_lf() {
    let repo = TempRepo::new("read");
    repo.write("crlf.txt", b"a\r\nb\r\n");
    repo.write("binario.dat", &[0u8, 1, 2, 3]);

    assert_eq!(tree::read_file(&repo.path, "crlf.txt", 10_000).unwrap(), "a\nb\n");
    assert!(tree::read_file(&repo.path, "binario.dat", 10_000).is_err());
    assert!(tree::read_file(&repo.path, "no-existe.txt", 10_000).is_err());
    assert!(tree::read_file(&repo.path, "../fuera.txt", 10_000).is_err());
}

#[test]
fn extrae_json_entre_texto_y_fences() {
    let plain = r#"{"files": ["a.ts"]}"#;
    assert!(extract_json(plain).is_some());

    let fenced = "Aqui tienes el plan:\n```json\n{\"edits\": [], \"summary\": \"ok\"}\n```\nGracias.";
    let value = extract_json(fenced).expect("debe extraer del bloque de codigo");
    assert_eq!(value.get("summary").and_then(|v| v.as_str()), Some("ok"));

    let drifted = "Texto previo {\"summary\": \"hola\", \"edits\": [],} texto posterior";
    assert!(extract_json(drifted).is_some(), "tolera comas finales");
}

#[test]
fn parsea_el_plan_con_alias_de_campos() {
    let camel = r###"{
        "summary": "Arregla el bug",
        "branchName": "agent/fix-bug",
        "commitMessage": "fix(auth): corrige validacion",
        "prTitle": "Corrige validacion",
        "prBody": "## Que cambia\nNada",
        "edits": [{"filePath": "src/a.ts", "searchContent": "x", "replaceContent": "y"}]
    }"###;

    let plan = parse_plan(camel).expect("plan camelCase");
    assert_eq!(plan.edits.len(), 1);
    assert_eq!(plan.edits[0].file_path, "src/a.ts");
    assert_eq!(plan.branch_name.as_deref(), Some("agent/fix-bug"));

    let snake = r#"{
        "summary": "otro",
        "edits": [{"file_path": "src/b.ts", "search_content": "a", "replace_content": "b"}]
    }"#;
    let plan = parse_plan(snake).expect("plan snake_case");
    assert_eq!(plan.edits[0].file_path, "src/b.ts");
    assert_eq!(plan.edits[0].search_content, "a");
}

#[test]
fn slug_de_rama_es_seguro() {
    assert_eq!(slugify("Corrige el bug de login!", 40), "corrige-el-bug-de-login");
    assert_eq!(slugify("    espacios   ", 40), "espacios");
    assert_eq!(slugify("!!!", 40), "tarea");
    let long = slugify(&"a".repeat(120), 20);
    assert!(long.chars().count() <= 20);
}

#[test]
fn detecta_texto_plano_en_ruta_inexistente_para_crear_archivo() {
    let repo = TempRepo::new("nuevo-dir");
    let blocks = vec![block("a/b/c/nuevo.txt", "", "hola\n")];
    let (changes, _, _) = apply(&repo.path, &blocks).expect("crear en subdirectorio");
    assert!(changes[0].created);
    assert!(Path::new(&repo.path).join("a/b/c/nuevo.txt").exists());
}

#[test]
fn archivo_sin_salto_final_se_anexa_correctamente() {
    let repo = TempRepo::new("sin-salto");
    repo.write("s.txt", b"linea");
    apply(&repo.path, &[block("s.txt", "", "nueva")]).expect("anexar");
    assert_eq!(repo.read("s.txt"), "linea\nnueva");
}

#[test]
fn archivo_binario_no_es_parcheable() {
    let repo = TempRepo::new("bin");
    repo.write("datos.bin", &[0u8, 159, 146, 150]);
    let error = apply(&repo.path, &[block("datos.bin", "a", "b")]).expect_err("debe fallar");
    assert!(error.contains("binario"), "mensaje: {}", error);
}

#[test]
fn detecta_ultimo_salto_disponible() {
    // Comprobacion trivial del helper usado en las pruebas anteriores.
    assert_eq!(find_last(b"abc\n", b'\n'), Some(3));
}

#[test]
fn avisa_cuando_el_parche_duplica_contenido_existente() {
    let repo = TempRepo::new("duplicado");
    repo.write("notas.md", b"# Notas\n\n## Contacto\nEscribe a dev@example.com.\n");

    // El modelo intenta insertar otra vez una seccion que ya esta.
    let blocks = vec![block(
        "notas.md",
        "Escribe a dev@example.com.",
        "Escribe a dev@example.com.\n\n## Contacto\nEscribe a dev@example.com.",
    )];

    let (_changes, _snapshots, warnings) = apply(&repo.path, &blocks).expect("aplicar");
    assert!(
        warnings.iter().any(|w| w.contains("Posible duplicado")),
        "debe avisar del duplicado: {:?}",
        warnings
    );
}

#[test]
fn no_avisa_de_duplicado_en_un_cambio_normal() {
    let repo = TempRepo::new("sin-duplicado");
    repo.write("notas.md", b"# Notas\n\nContenido.\n");

    let blocks = vec![block("notas.md", "Contenido.", "Contenido ampliado.")];
    let (_changes, _snapshots, warnings) = apply(&repo.path, &blocks).expect("aplicar");

    assert!(
        !warnings.iter().any(|w| w.contains("Posible duplicado")),
        "un cambio normal no debe avisar: {:?}",
        warnings
    );
}