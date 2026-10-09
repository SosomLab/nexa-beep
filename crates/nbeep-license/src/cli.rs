//! `nexa-beep --license …` — 라이선스 CLI(docs/50 D-33-7 · 출처: nexa-sql `crates/nsql-cli/src/license.rs`).
//!
//! ```text
//! nexa-beep --license status                       # 상태 · 파일 · 내용(기한) · 빌드일 · 기기 코드 · 설치 자리
//! nexa-beep --license request [<name> [<email>]]   # 요청 코드 한 줄(기기 ID는 해시 · 원문 없음) · 보낼 곳은 stderr
//! nexa-beep --license install <file>               # 검증 통과(Licensed)만 복사 · 원본 보존 · 무효면 쓰지 않음(종료 1)
//! nexa-beep --license remove                       # 사용자 폴더 파일 삭제(기기 공용 파일은 남는다)
//! nexa-beep --license path                         # 설치 자리
//! ```
//! 종료 코드: 0 · 1(실패·거부) · 2(사용법).
//!
//! 출력은 **영문 고정 키**(`state  licensed` 꼴 — 스크립트가 읽는다 · nexa-sql과 같은 모양). 데이터 폴더는 앱이 준다
//! (`data_dir()` — GUI와 같은 자리). 출력 대상은 주입(`out`·`err`)이라 시험이 프로세스 표준 출력을 건드리지 않는다.

use std::io::Write;
use std::path::Path;

use crate::{
    InstallError, License, LicenseState, Licensing, RequestMeta, LICENSE_CONTACT, PRODUCT,
};

/// 사용법(종료 2와 함께 stderr로).
pub const USAGE: &str = "usage: nexa-beep --license <status | request [<name> [<email>]] | install <file> | remove | path>\n";

/// `--license` 뒤의 인자(`args[0]` = 하위 명령)를 실행한다. 반환 = 프로세스 종료 코드.
pub fn run(args: &[String], data_dir: &Path, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let mut l = Licensing::open_default(data_dir);
    run_with(&mut l, args, out, err)
}

/// [`run`]의 본체 — `Licensing`을 주입받는다(시험 = 임시 폴더·임시 루트 키).
pub fn run_with(
    l: &mut Licensing,
    args: &[String],
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let arg = |i: usize| args.get(i).map(String::as_str);
    match arg(0) {
        Some("status" | "st") => status(l, out),
        Some("request" | "req") => request(arg(1), arg(2), out, err),
        Some("install" | "add") => match arg(1) {
            Some(f) => install(l, Path::new(f), out, err),
            None => usage(err),
        },
        Some("remove" | "rm") => remove(l, out, err),
        Some("path") => match l.primary_path() {
            Some(p) => {
                let _ = writeln!(out, "{}", p.display());
                0
            }
            None => {
                let _ = writeln!(err, "license folder is unknown");
                1
            }
        },
        _ => usage(err),
    }
}

fn usage(err: &mut dyn Write) -> i32 {
    let _ = err.write_all(USAGE.as_bytes());
    2
}

/// 상태 이름(로그·CLI 키) — 무효면 사유까지(`invalid(signature)`).
#[must_use]
pub fn state_label(s: &LicenseState) -> String {
    match s {
        LicenseState::Invalid(i) => format!("invalid({i:?})").to_ascii_lowercase(),
        other => other.name().to_string(),
    }
}

fn dash(s: &str) -> &str {
    if s.is_empty() {
        "-"
    } else {
        s
    }
}

fn print_license(l: &License, out: &mut dyn Write) {
    let _ = writeln!(out, "id             {}", l.id);
    let _ = writeln!(out, "licensee       {}", l.licensee);
    let _ = writeln!(out, "kind           {}", l.kind);
    let _ = writeln!(out, "tier           {}", l.tier);
    if !l.machines.is_empty() {
        let _ = writeln!(out, "machines       {}", l.machines.len());
    }
    let _ = writeln!(out, "issued         {}", l.issued);
    let _ = writeln!(out, "updates_until  {}", dash(&l.updates_until));
    let _ = writeln!(out, "expires        {}", dash(&l.expires));
    let _ = writeln!(
        out,
        "max_major      {}",
        l.max_major.map_or("-".to_string(), |m| format!("{m}.x"))
    );
    if !l.max_version.is_empty() {
        let _ = writeln!(out, "max_version    {}", l.max_version);
    }
}

fn status(l: &Licensing, out: &mut dyn Write) -> i32 {
    let _ = writeln!(out, "state          {}", state_label(l.state()));
    let _ = writeln!(
        out,
        "file           {}",
        l.path()
            .map_or("-".to_string(), |p| p.display().to_string())
    );
    if let Some(lic) = l.state().license() {
        print_license(lic, out);
    }
    let _ = writeln!(out, "build_date     {}", PRODUCT.build_date);
    let _ = writeln!(out, "app_version    {}", PRODUCT.version);
    let _ = writeln!(
        out,
        "machine        {}",
        Licensing::machine_code().unwrap_or_else(|| "-".to_string())
    );
    let _ = writeln!(
        out,
        "install_to     {}",
        l.primary_path()
            .map_or("-".to_string(), |p| p.display().to_string())
    );
    0
}

fn request(
    name: Option<&str>,
    email: Option<&str>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let meta = RequestMeta {
        name: name.unwrap_or("").to_string(),
        email: email.unwrap_or("").to_string(),
    };
    match Licensing::request_code(&meta) {
        Some(code) => {
            let _ = writeln!(out, "{code}");
            let _ = writeln!(err, "-> send to {LICENSE_CONTACT}");
            0
        }
        None => {
            let _ = writeln!(
                err,
                "cannot read this PC's machine ID (no OS machine identifier)"
            );
            1
        }
    }
}

fn install(l: &mut Licensing, file: &Path, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    match l.install(file) {
        Ok(lic) => {
            let _ = writeln!(
                out,
                "installed      {}",
                l.path()
                    .map_or("-".to_string(), |p| p.display().to_string())
            );
            print_license(&lic, out);
            0
        }
        Err(InstallError::Rejected(s)) => {
            let _ = writeln!(
                err,
                "license not installed: {} (source file untouched · machine = {})",
                state_label(s.as_ref()),
                Licensing::machine_code().unwrap_or_else(|| "-".to_string())
            );
            1
        }
        Err(InstallError::Io(e)) => {
            let _ = writeln!(err, "cannot install license: {e}");
            1
        }
    }
}

fn remove(l: &mut Licensing, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    match l.remove() {
        Ok(true) => {
            let _ = writeln!(out, "removed · state {}", state_label(l.state()));
            0
        }
        Ok(false) => {
            let _ = writeln!(out, "nothing to remove · state {}", state_label(l.state()));
            0
        }
        Err(e) => {
            let _ = writeln!(err, "cannot remove license: {e}");
            1
        }
    }
}
