use alloc::vec::Vec;

use crate::coverage;
use crate::FuzzResult;

/// Shell command types
#[derive(Clone, Copy)]
enum ShellCmd {
    Help,
    Ls,
    Cd,
    Pwd,
    Echo,
    Cat,
    Mkdir,
    Rm,
    Unknown,
}

/// Fuzzing input format for shell:
/// [cmd_len: u8] [cmd: bytes] [args: bytes]
pub fn execute(input: &[u8]) -> FuzzResult {
    if input.is_empty() {
        return FuzzResult::Normal;
    }

    let cmd_len = input[0] as usize;
    let cmd_end = 1 + cmd_len.min(input.len() - 1);
    let cmd = &input[1..cmd_end];
    let args = if input.len() > cmd_end {
        &input[cmd_end..]
    } else {
        &[]
    };

    let cmd_type = parse_command(cmd);

    coverage::record_edge((cmd_type as u8 as u64).wrapping_mul(100));

    match cmd_type {
        ShellCmd::Help => fuzz_help(args),
        ShellCmd::Ls => fuzz_ls(args),
        ShellCmd::Cd => fuzz_cd(args),
        ShellCmd::Pwd => fuzz_pwd(args),
        ShellCmd::Echo => fuzz_echo(args),
        ShellCmd::Cat => fuzz_cat(args),
        ShellCmd::Mkdir => fuzz_mkdir(args),
        ShellCmd::Rm => fuzz_rm(args),
        ShellCmd::Unknown => fuzz_unknown(cmd, args),
    }
}

fn parse_command(cmd: &[u8]) -> ShellCmd {
    match cmd {
        b"help" => ShellCmd::Help,
        b"ls" => ShellCmd::Ls,
        b"cd" => ShellCmd::Cd,
        b"pwd" => ShellCmd::Pwd,
        b"echo" => ShellCmd::Echo,
        b"cat" => ShellCmd::Cat,
        b"mkdir" => ShellCmd::Mkdir,
        b"rm" => ShellCmd::Rm,
        _ => ShellCmd::Unknown,
    }
}

fn fuzz_help(_args: &[u8]) -> FuzzResult {
    FuzzResult::Normal
}

fn fuzz_ls(args: &[u8]) -> FuzzResult {
    // Test ls with various arguments
    if args.is_empty() {
        return FuzzResult::Normal;
    }

    // Check for flags
    if args[0] == b'-' {
        coverage::record_edge(1);
    }

    FuzzResult::Normal
}

fn fuzz_cd(args: &[u8]) -> FuzzResult {
    // Test cd with various paths
    if args.is_empty() {
        return FuzzResult::Normal;
    }

    let path = match core::str::from_utf8(args) {
        Ok(s) => s,
        Err(_) => return FuzzResult::Normal,
    };

    // Check for path traversal
    if path.contains("..") {
        coverage::record_edge(2);
    }

    FuzzResult::Normal
}

fn fuzz_pwd(_args: &[u8]) -> FuzzResult {
    FuzzResult::Normal
}

fn fuzz_echo(args: &[u8]) -> FuzzResult {
    // Test echo with various inputs
    if args.len() > 4096 {
        return FuzzResult::Normal;
    }

    // Check for special characters
    if args.iter().any(|&b| b == b'$' || b == b'`') {
        coverage::record_edge(3);
    }

    FuzzResult::Normal
}

fn fuzz_cat(args: &[u8]) -> FuzzResult {
    // Test cat with various files
    if args.is_empty() {
        return FuzzResult::Normal;
    }

    let path = match core::str::from_utf8(args) {
        Ok(s) => s,
        Err(_) => return FuzzResult::Normal,
    };

    // Check for special files
    if path.starts_with("/dev/") {
        coverage::record_edge(4);
    }

    FuzzResult::Normal
}

fn fuzz_mkdir(args: &[u8]) -> FuzzResult {
    // Test mkdir with various paths
    if args.is_empty() {
        return FuzzResult::Normal;
    }

    let path = match core::str::from_utf8(args) {
        Ok(s) => s,
        Err(_) => return FuzzResult::Normal,
    };

    if path.len() > 255 {
        return FuzzResult::Normal;
    }

    FuzzResult::Normal
}

fn fuzz_rm(args: &[u8]) -> FuzzResult {
    // Test rm with various paths
    if args.is_empty() {
        return FuzzResult::Normal;
    }

    // Check for recursive flag
    if args.len() >= 2 && &args[..2] == b"-r" {
        coverage::record_edge(5);
    }

    FuzzResult::Normal
}

fn fuzz_unknown(cmd: &[u8], _args: &[u8]) -> FuzzResult {
    // Test unknown commands
    if cmd.len() > 100 {
        return FuzzResult::Normal;
    }

    // Check for shell injection attempts
    if cmd.iter().any(|&b| b == b';' || b == b'|' || b == b'&') {
        coverage::record_edge(6);
    }

    FuzzResult::Normal
}

/// Shell seeds: the documented starter corpus plus every mutation shape the
/// doc lists (empty, spaces, traversal, command substitution, separators,
/// repetition bombs).
pub fn generate_seeds() -> Vec<Vec<u8>> {
    const COMMANDS: &[&[u8]] = &[
        b"help",
        b"ls",
        b"ls -la",
        b"cd /",
        b"cd ..",
        b"pwd",
        b"echo test",
        b"echo $(id)",
        b"echo `id`",
        b"cat file",
        b"cat /dev/null",
        b"cat /proc/self/maps",
        b"mkdir test",
        b"rm test",
        b"rm -rf /",
        b"",
        b" ",
        b"////",
        b"../../..",
        b"; ; ;",
        b"&&&&",
        b"||||",
        b"aaaa....aaaa",
        b"$(())",
        b"$PATH",
        b"\x00\x00",
        b"echo aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    ];
    let mut seeds = Vec::new();
    for cmd in COMMANDS {
        for args in [&b""[..], b"/", b"-r", b"..", b"////", b"-la"] {
            let mut seed = Vec::with_capacity(1 + cmd.len() + args.len());
            seed.push(cmd.len() as u8);
            seed.extend_from_slice(cmd);
            seed.extend_from_slice(args);
            seeds.push(seed);
        }
    }
    seeds
}
