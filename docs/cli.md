# MAGI CLI Reference

## Usage

```
magi <command> [options] [arguments]
```

## Commands

### Running Code
| Command | Description |
|---------|-------------|
| `magi run <file>` | Execute a .magi file |
| `magi run --watch <file>` | Re-run on file changes |
| `magi run --sandbox <file>` | Execute in sandboxed mode |
| `magi run --timeout <secs> <file>` | Execute with timeout |
| `magi run --json <file>` | Output as JSON |
| `magi run-bc <file>` | Execute using bytecode VM |
| `magi compilec <file>` | Compile to .magc classfile |
| `magi runc <file.magc>` | Execute .magc on MagiVM runtime |
| `magi eval '<expr>'` | Evaluate an expression |
| `magi repl` | Interactive REPL |

### Building & Compiling
| Command | Description |
|---------|-------------|
| `magi build` | Build the project |
| `magi compile <file>` | Compile to WASM |
| `magi compile-native <file>` | Compile to native ELF x86-64 executable |
| `magi run-wasm <file.wasm>` | Run compiled WASM |
| `magi expand <file>` | Show expanded/formatted AST |

### Testing
| Command | Description |
|---------|-------------|
| `magi test <file>` | Run `#[test]` functions |
| `magi test --filter <pat> <file>` | Run matching tests |
| `magi test --timeout <ms> <file>` | Per-test timeout |
| `magi test-all` | Run all tests in project |
| `magi doc-test <file>` | Run doc comment examples |
| `magi bench <file>` | Benchmark execution |
| `magi coverage <file>` | Show test coverage |

### Code Quality
| Command | Description |
|---------|-------------|
| `magi check <file>` | Type-check without running |
| `magi lint <file>` | Run linter |
| `magi fmt <file>` | Check formatting |
| `magi fmt --write <file>` | Auto-format in place |
| `magi fix <file>` | Auto-fix lint issues |

### Documentation
| Command | Description |
|---------|-------------|
| `magi doc <file>` | Generate documentation |
| `magi doc-test <file>` | Run doc examples |

### Debugging
| Command | Description |
|---------|-------------|
| `magi debug <file>` | Step-through debugger |

### Package Management
| Command | Description |
|---------|-------------|
| `magi init <name>` | Create new project |
| `magi get` | Fetch dependencies |
| `magi add <package>` | Add dependency |
| `magi remove <package>` | Remove dependency |
| `magi install <url>` | Install globally |
| `magi uninstall <package>` | Remove global install |
| `magi publish` | Publish package |
| `magi update` | Update dependencies |
| `magi audit` | Security audit |
| `magi vendor` | Vendor dependencies |
| `magi tree` | Show dependency tree |
| `magi search <query>` | Search packages/stdlib |

### Analysis
| Command | Description |
|---------|-------------|
| `magi bloat` | Analyze binary size |
| `magi trace` | Execution tracing |
| `magi generate` | Run code generation |
| `magi workspace` | Workspace management |
| `magi vm-stats [file]` | Show VM and GC statistics |

### Servers
| Command | Description |
|---------|-------------|
| `magi lsp` | Start LSP server (JSON-RPC over stdio) |
| `magi mcp` | Start MCP server (Model Context Protocol over stdio) |

### Other
| Command | Description |
|---------|-------------|
| `magi env` | Print environment |
| `magi clean` | Remove build artifacts |
| `magi version` | Print version |
| `magi help` | Print help |

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `MAGI_HOME` | `~/.magi` | MAGI home directory |
| `MAGI_PATH` | `$MAGI_HOME/packages` | Package search path |
| `MAGI_ROOT` | (auto) | MAGI installation root |
| `MAGI_BIN` | `$MAGI_HOME/bin` | Binary install directory |
| `MAGI_CACHE` | `$MAGI_HOME/cache` | Build cache directory |
| `MAGI_MODCACHE` | `$MAGI_HOME/mod` | Module cache directory |
| `MAGI_PROXY` | `direct` | Package proxy URL |
| `MAGI_PRIVATE` | (empty) | Private module patterns |
| `MAGI_FLAGS` | (empty) | Default CLI flags |
| `MAGI_LOG` | (empty) | Log level |
| `MAGI_BACKTRACE` | `0` | Enable backtraces (`1` = on) |
| `MAGI_INCREMENTAL` | `1` | Incremental compilation |
| `MAGI_TARGET` | `target` | Build output directory |
| `MAGI_TOOLCHAIN` | `default` | Toolchain selection |
| `MAGI_ARCH` | (auto) | Target architecture |
| `MAGI_OS` | (auto) | Target OS |
| `MAGI_VERSION` | (auto) | Current MAGI version |
| `MAGI_CWD` | (auto) | Current working directory |

## REPL Commands

| Command | Description |
|---------|-------------|
| `:help` | Show help |
| `:quit` | Exit |
| `:type <expr>` | Show expression type |
| `:time <expr>` | Time execution |
| `:load <file>` | Load and execute file |
| `:clear` | Reset state |
| `:save` | Save session history |
| `:search <pat>` | Search history |
