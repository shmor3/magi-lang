# MAGI Standard Library Reference

105 modules, 1355 total callable operations and functions.

Usage: `use std::module_name::*;`

---

### `array` (20)
`array_get`, `array_set`, `array_push`, `array_pop`, `array_shift`, `array_length`, `array_slice`, `array_concat`, `array_contains`, `array_sort`, `array_reverse`, `array_flatten`, `array_filter_nulls`, `array_join`, `array_unique`, `array_insert`, `array_remove`, `array_from_map`, `reduce`, `range`

### `array_ext` (4)
`chunks_exact`, `rotate_array`, `drain`, `swap_remove`

### `big` (3)
`big_int`, `big_int_add`, `big_int_mul`


### `binary` (4)
`binary_encode`, `binary_decode`, `encoding_binary_read`, `encoding_binary_write`

### `bits` (6)
`bit_and`, `bit_or`, `bit_xor`, `bit_not`, `bit_shift_left`, `bit_shift_right`

### `bloom` (3)
`bloom_filter_new`, `bloom_add`, `bloom_contains`

### `bufio` (3)
`bufio_scanner`, `bufio_reader`, `bufio_writer`


### `build` (4)
`build_cache_enabled`, `build_target`, `build_features`, `build_incremental`

### `bytes` (17)
`bytes_length`, `bytes_slice`, `bytes_concat`, `bytes_contains`, `base64_encode`, `base64_decode`, `bytes_compare`, `bytes_equal`, `bytes_has_prefix`, `bytes_has_suffix`, `bytes_index`, `bytes_join`, `bytes_repeat`, `bytes_split`, `bytes_trim`, `bytes_from_string`, `bytes_to_string`

### `cert` (6)
`cert_generate`, `cert_parse`, `cert_info`, `cert_verify`, `key_generate`, `cert_self_signed`

### `cmp` (6)
`equal`, `not_equal`, `greater`, `less`, `greater_eq`, `less_eq`

### `collections` (9)
`set_from`, `set_union`, `set_intersection`, `set_difference`, `set_symmetric_difference`, `counter`, `most_common`, `ordered_map`, `from_entries`

### `complex` (2)
`complex_new`, `complex_abs`

### `compress` (14)
`compress_zstd`, `decompress_zstd`, `compress_lz4`, `decompress_lz4`, `compress_gzip`, `decompress_gzip`, `tar_create`, `tar_list`, `zip_create`, `zip_list`, `compress_zlib`, `decompress_zlib`, `compress_snappy`, `decompress_snappy`

### `concurrent` (39)
`channel`, `chan_send`, `chan_recv`, `chan_try_recv`, `chan_close`, `select`, `once_new`, `once_call`, `atomic_new`, `atomic_load`, `atomic_store`, `atomic_add`, `atomic_cas`, `wait_group_new`, `wait_group_add`, `wait_group_done`, `wait_group_wait`, `rwlock_new`, `rwlock_read`, `rwlock_write`, `timer_new`, `timer_after`, `timer_stop`, `ticker_new`, `ticker_stop`, `context_new`, `context_cancel`, `context_is_done`, `context_with_timeout`, `rate_limiter_new`, `rate_limiter_allow`, `thread_pool_new`, `thread_pool_submit`, `condvar_new`, `condvar_signal`, `condvar_broadcast`, `condvar_wait`, `sync_map`, `sync_pool`

### `container` (5)
`priority_queue_new`, `ring_buffer_new`, `lru_cache_new`, `linked_list_new`, `deque_new`

### `control` (10)
`if_else`, `switch`, `coalesce`, `try_catch`, `error`, `option_map`, `result_map`, `result_map_err`, `iota`, `iota_reset`


### `convert` (21)
`to_string`, `to_int64`, `to_float64`, `to_bool`, `to_bytes`, `from_bytes`, `parse_json`, `to_json`, `parse_int`, `parse_float`, `typeof`, `default`, `is_null`, `is_string`, `is_number`, `is_array`, `is_map`, `is_bool`, `is_bytes`, `char_from_code`, `char_code`

### `crypto` (23)
`hash_sha512`, `hmac_sha256`, `hash_crc32`, `constant_time_eq`, `aes_encrypt`, `aes_decrypt`, `csprng`, `pbkdf2`, `bcrypt_hash`, `bcrypt_verify`, `rsa_generate_key`, `rsa_sign`, `rsa_verify`, `ecdsa_generate_key`, `ecdsa_sign`, `ecdsa_verify`, `ed25519_generate_key`, `ed25519_sign`, `ed25519_verify`, `chacha20_encrypt`, `chacha20_decrypt`, `argon2_hash`, `hkdf`

### `crypto_ext` (1)
`constant_time_compare`

### `csv` (4)
`csv_parse`, `csv_stringify`, `csv_headers`, `csv_parse_rows`

### `database` (5)
`db_open`, `db_get`, `db_set`, `db_delete`, `db_close`


### `debug` (3)
`debug_buildinfo`, `debug_stack`, `debug_tasks`

### `embed` (2)
`embed_file`, `embed_string`

### `encode` (4)
`html_escape`, `html_unescape`, `base32_encode`, `base32_decode`

### `encoding_ext` (5)
`qp_encode`, `qp_decode`, `ascii85_encode`, `ini_parse`, `ini_stringify`

### `env` (13)
`env_get`, `env_set`, `env_has`, `env_keys`, `env_vars`, `os_name`, `os_arch`, `process_pid`, `current_dir`, `hostname`, `user_home_dir`, `executable_path`, `getuid`

### `errors` (7)
`error_new`, `error_wrap`, `error_unwrap`, `error_is`, `error_chain`, `errors_join`, `errors_as`

### `expvar` (2)
`expvar_set`, `expvar_get`

### `ffi` (4)
`ffi_load_library`, `ffi_call`, `ffi_symbol`, `ffi_close`

### `file_ops` (2)
`file_lock`, `file_unlock`

### `flag` (2)
`flag_parse`, `flag_args`


### `fmt` (8)
`fmt_number`, `fmt_bytes`, `fmt_duration`, `fmt_hex`, `fmt_binary`, `fmt_percent`, `sprintf`, `tabwriter`

### `fmt_ext` (1)
`errorf`

### `fs` (29)
`fs_read`, `fs_write`, `fs_append`, `fs_exists`, `fs_remove`, `fs_list`, `fs_mkdir`, `fs_copy`, `fs_move`, `fs_size`, `fs_is_file`, `fs_is_dir`, `fs_chmod`, `fs_symlink`, `fs_readlink`, `fs_watch`, `file_metadata`, `glob`, `mkdir_all`, `temp_dir`, `temp_file`, `chmod`, `chown`, `symlink`, `hardlink`, `readlink`, `file_seek`, `file_lock`, `pipe`

### `gob` (2)
`gob_encode`, `gob_decode`

### `hash` (12)
`hash_sha256`, `hash_sha1`, `hash_blake3`, `hash_md5`, `url_encode`, `url_decode`, `hex_encode`, `hex_decode`, `hash_sha512`, `hmac_sha256`, `hash_crc32`, `constant_time_eq`

### `hash_ext` (4)
`hash_adler32`, `hash_fnv32`, `hash_fnv64`, `hash_crc64`

### `http_helpers` (10)
`form_encode`, `basic_auth`, `bearer_auth`, `http_multipart_upload`, `http_cookie_jar`, `http_redirect_follow`, `http_proxy`, `http_streaming_response`, `http_connection_pool`, `wss_connect`

### `http_server` (4)
`http_server_start`, `http_server_receive`, `http_server_respond`, `http_server_stop`

### `image` (1)
`image_info`

### `io` (7)
`debug_log`, `assert`, `error`, `file_seek`, `io_copy`, `io_read_all`, `io_pipe`


### `io_ext` (2)
`limit_reader`, `cursor`

### `itertools` (5)
`iter_chain`, `iter_cycle`, `iter_repeat`, `iter_product`, `iter_pairwise`

### `json` (14)
`json_get`, `json_set`, `json_delete`, `json_flatten`, `json_merge`, `json_type`, `json_validate`, `json_pretty_print`, `json_compact`, `json_query`, `json_diff`, `json_patch`, `json_schema_validate`, `json_stream_parse`

### `log` (3)
`log_println`, `log_fatal`, `log_panic`


### `logic` (4)
`and`, `or`, `not`, `xor`


### `map` (11)
`map_get`, `map_set`, `map_delete`, `map_has`, `map_keys`, `map_values`, `map_entries`, `map_merge`, `map_size`, `map_from_entries`, `map_update`

### `math` (82)
`add`, `subtract`, `multiply`, `divide`, `modulo`, `power`, `sqrt`, `cbrt`, `hypot`, `abs`, `negate`, `min`, `max`, `round`, `floor`, `ceil`, `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `atan2`, `sinh`, `cosh`, `tanh`, `log`, `ln`, `log2`, `log10`, `exp`, `to_radians`, `to_degrees`, `clamp`, `lerp`, `remap`, `sign`, `gcd`, `lcm`, `is_nan`, `is_infinite`, `is_finite`, `approx_eq`, `math_sum`, `math_product`, `math_average`, `math_min_of`, `math_max_of`, `math_count`, `factorial`, `fibonacci`, `is_prime`, `ncr`, `npr`, `combinations`, `permutations`, `math_pi`, `math_e`, `math_inf`, `math_neg_inf`, `math_nan`, `math_tau`, `math_ln2`, `math_ln10`, `math_sqrt2`, `copysign`, `dim`, `remainder`, `pow10`, `round_to_even`, `frexp`, `ldexp`, `logb`, `math_gamma`, `math_erf`, `gamma`, `lgamma`, `erf`, `erfc`, `expm1`, `nextafter`, `signbit`

### `math_bits` (2)
`bit_len`, `reverse_bytes`

### `mime` (1)
`mime_type`

### `msgpack` (2)
`messagepack_encode`, `messagepack_decode`

### `net` (15)
`http_get`, `http_post`, `http_put`, `http_delete`, `http_request`, `http_head`, `http_options`, `http_patch`, `url_parse`, `url_join`, `dns_lookup`, `unix_socket_connect`, `multipart_encode`, `cookie_parse`, `cookie_format`

### `net_ext` (4)
`resolve_tcp_addr`, `lookup_host`, `lookup_cname`, `interface_addrs`

### `netip` (2)
`parse_ip`, `parse_email_address`

### `os_ext` (2)
`clearenv`, `unsetenv`

### `os_ext2` (5)
`chtimes`, `getpagesize`, `is_exist`, `lookup_env`, `user_cache_dir`

### `path` (15)
`path_join`, `path_basename`, `path_dirname`, `path_extension`, `path_stem`, `path_is_absolute`, `path_normalize`, `path_split`, `path_with_extension`, `path_parent`, `cwd`, `chdir`, `path_abs`, `path_rel`, `path_walk`

### `plugin` (2)
`plugin_open`, `plugin_lookup`

### `pprof` (3)
`pprof_start_cpu`, `pprof_stop_cpu`, `pprof_write_heap`

### `process` (6)
`exit`, `pid`, `exec`, `exec_output`, `exec_status`, `trap_sigint`

### `profiling` (3)
`cpu_profile`, `memory_profile`, `flamegraph`

### `protobuf` (4)
`protobuf_encode`, `proto_encode`, `protobuf_decode`, `proto_decode`

### `rand` (10)
`random_int`, `random_float`, `random_bool`, `random_bytes`, `random_range`, `random_choice`, `random_shuffle`, `random_sample`, `random_uuid`, `random_string`

### `reflect` (13)
`reflect_type_of`, `reflect_type_name`, `reflect_is_type`, `reflect_fields`, `reflect_has_field`, `reflect_callable`, `reflect_arity`, `reflect_inspect`, `enum_values`, `enum_variant_name`, `struct_fields`, `struct_name`, `struct_has_field`

### `reflect_ext` (2)
`type_size`, `type_align`

### `regex` (5)
`regex_split`, `regex_escape`, `regex_test`, `regex_captures`, `regex_find_all`

### `runtime` (6)
`runtime_tasks`, `runtime_mem_usage`, `runtime_num_cpu`, `runtime_num_tasks`, `runtime_gc`, `runtime_mem_stats`

### `collections_ext` (4)
`btreemap_new`, `btreeset_new`, `binary_heap_new`, `vecdeque_new`

### `iter_ext` (3)
`fuse`, `advance_by`, `collect_into`

### `option_ext` (1)
`map_or_else`

### `path_ext` (3)
`ancestors`, `with_file_name`, `soft_link`

### `result_ext` (1)
`inspect_err`

### `string_ext` (2)
`make_ascii_uppercase`, `make_ascii_lowercase`

### `sync_ext` (4)
`barrier_new`, `barrier_wait`, `mpsc_channel`, `mpsc_sync_channel`

### `thread_ext` (3)
`yield_now`, `thread_park`, `thread_unpark`

### `vec_ext` (4)
`dedup_by`, `dedup_by_key`, `split_off`, `binary_search_by_key`

### `security` (3)
`global_execution_timeout`, `memory_limit`, `secrets_redaction`

### `slog` (1)
`slog`


### `sort` (9)
`sort_asc`, `sort_desc`, `sort_by`, `sort_by_key`, `stable_sort`, `is_sorted`, `binary_search`, `binary_search_by`, `sort_reverse`

### `sql` (9)
`sql_open`, `sql_exec`, `sql_query`, `sql_query_row`, `sql_begin`, `sql_commit`, `sql_rollback`, `sql_close`, `sql_prepare`

### `sse` (3)
`sse_connect`, `sse_read_event`, `sse_close`


### `stats` (12)
`stats_mean`, `stats_median`, `stats_mode`, `stats_variance`, `stats_std_dev`, `stats_min_by`, `stats_max_by`, `stats_sum`, `stats_percentile`, `stats_quantile`, `stats_covariance`, `stats_correlation`

### `str` (44)
`concat`, `split`, `substring`, `length`, `replace`, `to_upper`, `to_lower`, `trim`, `trim_start`, `trim_end`, `contains`, `starts_with`, `ends_with`, `char_at`, `index_of`, `pad_start`, `pad_end`, `string_repeat`, `string_reverse`, `string_lines`, `string_words`, `string_count`, `string_chars`, `string_join`, `string_template`, `string_format`, `regex_match`, `regex_replace`, `regex_extract`, `encode`, `contains_rune`, `cut_prefix`, `cut_suffix`, `index_byte`, `index_rune`, `index_func`, `last_index_func`, `last_index_any`, `last_index_byte`, `split_after_n`, `trim_func`, `trim_left_func`, `trim_right_func`, `to_valid_utf8`

### `strconv` (12)
`format_bool`, `format_int`, `format_float`, `parse_bool`, `parse_uint`, `from_str_radix`, `split_after`, `index_any`, `fields_func`, `is_graphic`, `is_print`, `can_backquote`

### `strings_builder` (3)
`string_builder`, `sb_write`, `sb_string`

### `strings_ext` (5)
`to_title`, `to_valid_utf8`, `split_after`, `index_any`, `fields_func`

### `syscall` (3)
`syscall_exec`, `syscall_getenv`, `syscall_setenv`

### `tabwriter` (1)
`tabwriter_format`

### `tcp` (7)
`tcp_connect`, `tcp_write`, `tcp_read`, `tcp_close`, `tcp_bind`, `tcp_accept`, `tcp_server_close`

### `template` (1)
`template_render`


### `testing` (11)
`assert`, `assert_eq`, `assert_ne`, `assert_throws`, `assert_timeout`, `skip_test`, `subtest`, `test_parallel`, `test_coverage`, `test_fuzz`, `test_snapshot`

### `text` (10)
`text_wrap`, `text_dedent`, `text_indent`, `text_pad_left`, `text_pad_right`, `text_truncate`, `text_slug`, `text_camel_case`, `text_snake_case`, `text_title_case`

### `text_template` (2)
`go_template`, `html_template_render`

### `time` (34)
`now_timestamp`, `format_timestamp`, `parse_timestamp`, `timestamp_add`, `timestamp_diff`, `sleep`, `duration`, `elapsed`, `time_sleep`, `add_duration`, `sub_duration`, `time_diff`, `start_of`, `end_of`, `date_now`, `date_parse`, `date_format`, `date_add`, `date_diff`, `duration_ms`, `duration_secs`, `duration_mins`, `duration_hours`, `time_unix`, `time_unix_milli`, `time_since`, `time_until`, `time_date`, `time_unix_nano`, `time_format_rfc3339`, `time_parse_duration`, `time_format_duration`, `time_after_func`, `monotonic_now`

### `toml` (2)
`toml_parse`, `toml_stringify`


### `udp` (4)
`udp_bind`, `udp_send_to`, `udp_recv_from`, `udp_close`


### `unicode` (7)
`is_letter`, `is_digit`, `is_space`, `is_upper`, `is_lower`, `is_printable`, `char_category`

### `utf16` (2)
`utf16_encode`, `utf16_decode`

### `uuid` (4)
`uuid_v4`, `uuid_parse`, `uuid_is_valid`, `uuid_nil`


### `validate` (4)
`is_email`, `is_ipv4`, `is_ipv6`, `is_url`

### `ws` (4)
`ws_connect`, `ws_send`, `ws_receive`, `ws_close`


### `yaml` (6)
`yaml_parse`, `yaml_stringify`, `yaml_validate`, `yaml_to_json`, `yaml_from_json`, `yaml_merge`

### `string_builder` (4)
`string_builder_new`, `string_builder_append`, `string_builder_to_string`, `string_builder_len`

### `platform` (31)
**Terminal (termios FFI):** `raw_mode_enable`, `raw_mode_disable`, `read_byte`, `read_byte_timeout`
**SDL2 Graphics (optional):** `sdl_init`, `sdl_set_color`, `sdl_clear`, `sdl_present`, `sdl_draw_pixel`, `sdl_draw_line`, `sdl_fill_rect`, `sdl_poll_event`, `sdl_delay`, `sdl_ticks`, `sdl_destroy`
**Audio (PulseAudio FFI, optional):** `audio_stream_new`, `audio_write_samples`, `audio_drain`, `audio_close`
**WebGPU (WASM target):** `gpu_init`, `gpu_create_buffer`, `gpu_create_shader`, `gpu_create_pipeline`, `gpu_begin_render_pass`, `gpu_draw`, `gpu_end_render_pass`, `gpu_submit`, `gpu_present`, `gpu_write_buffer`, `gpu_create_texture`, `gpu_destroy`

