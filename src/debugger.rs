//! MAGI Debugger — step-through execution with breakpoints and variable inspection.
//!
//! Usage: `magi debug file.magi`

use crate::syntax::ast::*;
use crate::syntax::interpreter::Interpreter;
use crate::eval::OperationEvaluator;
use std::collections::HashSet;
use std::io::{self, Write};

/// Run a MAGI program in debug mode.
/// Parses the source, then executes statement-by-statement with a debug prompt.
pub fn debug_run(source: &str, evaluator: &dyn OperationEvaluator) {
    let program = match crate::syntax::parser::parse_v2(source) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Parse error: {}", e.message);
            return;
        }
    };

    let source_lines: Vec<&str> = source.lines().collect();
    let mut breakpoints: HashSet<u32> = HashSet::new();
    let mut step_mode = true;

    println!("MAGI Debugger");
    println!("Type 'h' for help. Stepping through program.");
    println!();

    let mut interp = Interpreter::new(evaluator);

    // Pass 1: register definitions
    for stmt in &program.statements {
        match &stmt.kind {
            StatementKind::FunctionDef(_) | StatementKind::AsyncFunctionDef(_) |
            StatementKind::EnumDef { .. } | StatementKind::StructDef { .. } |
            StatementKind::TraitDef { .. } | StatementKind::ImplBlock { .. } |
            StatementKind::ImplTrait { .. } | StatementKind::ModuleDef { .. } => {
                // Register via execute (it handles definitions in pass 1)
                let _ = interp.execute(&Program { statements: vec![stmt.clone()], span: stmt.span, trailing_comments: vec![] });
            }
            _ => {}
        }
    }

    // Pass 2: execute with debug control
    for stmt in &program.statements {
        let line = stmt.span.start_line;

        match &stmt.kind {
            StatementKind::FunctionDef(_) | StatementKind::AsyncFunctionDef(_) |
            StatementKind::EnumDef { .. } | StatementKind::StructDef { .. } |
            StatementKind::TraitDef { .. } | StatementKind::ImplBlock { .. } |
            StatementKind::ImplTrait { .. } | StatementKind::ModuleDef { .. } => continue,
            _ => {}
        }

        if step_mode || breakpoints.contains(&line) {
            // Show context
            let l = line as usize;
            let start = l.saturating_sub(2);
            let end = (l + 2).min(source_lines.len());
            for i in start..end {
                let marker = if i + 1 == l { ">>>" } else { "   " };
                println!("{} {:>4} | {}", marker, i + 1, source_lines.get(i).unwrap_or(&""));
            }

            // Debug prompt
            loop {
                print!("(dbg) ");
                let _ = io::stdout().flush();
                let mut input = String::new();
                if io::stdin().read_line(&mut input).unwrap_or(0) == 0 {
                    return;
                }
                let cmd = input.trim();
                match cmd {
                    "s" | "step" | "n" | "next" | "" => { step_mode = true; break; }
                    "c" | "continue" => { step_mode = false; break; }
                    "q" | "quit" => return,
                    "h" | "help" => {
                        println!("  s/step  — execute one statement");
                        println!("  c       — continue to next breakpoint");
                        println!("  b <N>   — set breakpoint at line N");
                        println!("  d <N>   — delete breakpoint at line N");
                        println!("  w       — show current location");
                        println!("  q       — quit");
                    }
                    "w" | "where" => {
                        for i in start..end {
                            let marker = if i + 1 == l { ">>>" } else { "   " };
                            println!("{} {:>4} | {}", marker, i + 1, source_lines.get(i).unwrap_or(&""));
                        }
                    }
                    _ if cmd.starts_with("b ") => {
                        if let Ok(n) = cmd[2..].trim().parse::<u32>() {
                            breakpoints.insert(n);
                            println!("Breakpoint at line {}", n);
                        }
                    }
                    _ if cmd.starts_with("d ") => {
                        if let Ok(n) = cmd[2..].trim().parse::<u32>() {
                            breakpoints.remove(&n);
                            println!("Removed breakpoint at line {}", n);
                        }
                    }
                    _ => println!("Unknown command. Type 'h' for help."),
                }
            }
        }

        // Execute the single statement
        let mini_program = Program { statements: vec![stmt.clone()], span: stmt.span, trailing_comments: vec![] };
        match interp.execute(&mini_program) {
            Ok(_) => {
                for log in interp.logs.drain(..) {
                    println!("{}", log.message);
                }
            }
            Err(e) => {
                eprintln!("Error at line {}: {}", line, e);
            }
        }
    }
    println!("Program finished.");
}
