//! Operation type mapping: output types and input port types.

use crate::types::{ChannelType, OperationType};

/// Returns the output `ChannelType` an operation produces.
pub fn op_output_type(op: OperationType) -> ChannelType {
    use OperationType::*;
    match op {
        // Arithmetic — output depends on inputs (Null port = accepts any type)
        Add | Subtract | Multiply | Modulo | Power | Sqrt | Cbrt | Hypot | Negate | Abs | Min | Max | Round
        | Floor | Ceil | Divide => ChannelType::Null,

        // Comparison → bool
        Equal | NotEqual | Greater | Less | GreaterEq | LessEq => ChannelType::Bool,

        // Logic → bool
        And | Or | Not | Xor => ChannelType::Bool,

        // Bitwise — output depends on inputs
        BitAnd | BitOr | BitXor | BitNot | BitShiftLeft | BitShiftRight => ChannelType::Null,

        // Control flow — polymorphic
        IfElse | Switch | Coalesce | TryCatch => ChannelType::Null,
        Assert => ChannelType::Bool,
        DebugLog => ChannelType::Null,
        Error => ChannelType::Null,

        // V2 Language Constructs — polymorphic
        FunctionDef | FunctionCall | AsyncSpawn | AsyncAwait | LoopGroup => ChannelType::Null,

        // String → string
        Concat | Replace | Substring | ToUpper | ToLower | Trim | TrimStart | TrimEnd
        | PadStart | PadEnd | StringReverse | StringRepeat | StringFormat | StringJoin
        | StringTemplate => ChannelType::String,
        Split | StringLines | StringWords => ChannelType::Array,
        Length | IndexOf | StringCount => ChannelType::Int64,
        Contains | StartsWith | EndsWith | RegexMatch => ChannelType::Bool,
        CharAt => ChannelType::String,
        RegexReplace => ChannelType::String,
        RegexExtract => ChannelType::Array,

        // Type conversion — output matches target type
        ToString => ChannelType::String,
        ToInt64 => ChannelType::Int64,
        ToFloat64 => ChannelType::Float64,
        ToBool => ChannelType::Bool,
        ToBytes => ChannelType::Bytes,
        FromBytes => ChannelType::String,
        ParseJson => ChannelType::Null,
        ToJson => ChannelType::String,
        CharFromCode => ChannelType::String,
        CharCode => ChannelType::Int64,
        Typeof => ChannelType::String,
        Default => ChannelType::Null,

        // Array → various
        ArrayGet => ChannelType::Null,
        ArraySet | ArrayPush | ArrayConcat | ArrayReverse | ArrayFlatten | ArraySort
        | ArrayFilterNulls | ArrayInsert | ArrayRemove => ChannelType::Array,
        ArrayPop | ArrayShift => ChannelType::Null, // returns element (any type)
        ArrayFromMap => ChannelType::Array,
        ArrayLength => ChannelType::Int64,
        ArraySlice => ChannelType::Array,
        ArrayContains => ChannelType::Bool,
        ArrayJoin => ChannelType::String,
        Range => ChannelType::Array,
        Reduce => ChannelType::Null,

        // Map → various
        MapGet => ChannelType::Null,
        MapSet | MapDelete | MapMerge | MapFromEntries | MapUpdate => ChannelType::Map,
        MapHas => ChannelType::Bool,
        MapKeys | MapValues | MapEntries => ChannelType::Array,
        MapSize => ChannelType::Int64,

        // Bytes → various
        BytesLength => ChannelType::Int64,
        BytesSlice | BytesConcat | BytesJoin | BytesRepeat | BytesTrim | BytesFromString => ChannelType::Bytes,
        BytesContains | BytesEqual | BytesHasPrefix | BytesHasSuffix => ChannelType::Bool,
        BytesCompare | BytesIndex => ChannelType::Int64,
        BytesSplit => ChannelType::Array,
        BytesToString => ChannelType::String,
        Base64Encode | HexEncode => ChannelType::String,
        Base64Decode | HexDecode => ChannelType::Bytes,

        // Errors → various
        ErrorNew | ErrorWrap => ChannelType::Map,
        ErrorUnwrap => ChannelType::Null,
        ErrorIs => ChannelType::Bool,
        ErrorChain => ChannelType::Array,

        // JSON → various
        JsonGet => ChannelType::Null,
        JsonSet | JsonDelete | JsonMerge => ChannelType::Null,
        JsonFlatten => ChannelType::Map,
        JsonType => ChannelType::String,
        JsonValidate => ChannelType::Bool,
        JsonPrettyPrint | JsonCompact => ChannelType::String,
        JsonQuery => ChannelType::Null,

        // DateTime
        NowTimestamp => ChannelType::Int64,
        FormatTimestamp => ChannelType::String,
        ParseTimestamp => ChannelType::Int64,
        TimestampAdd => ChannelType::Int64,
        TimestampDiff => ChannelType::Int64,
        Sleep => ChannelType::Null,

        // Hash/Encode → string
        HashSha256 | HashSha1 | HashBlake3 | HashMd5 | UrlEncode | UrlDecode => ChannelType::String,

        // Array Higher-Order
        ArrayMap | ArrayFlatMap | ArrayScan => ChannelType::Array,
        ArrayFilter | ArrayTakeWhile | ArraySkipWhile => ChannelType::Array,
        ArrayFind => ChannelType::Null,
        ArrayFindIndex => ChannelType::Int64,
        ArrayEvery | ArraySome => ChannelType::Bool,
        ArrayZip | ArrayEnumerate => ChannelType::Array,
        ArrayTake | ArraySkip => ChannelType::Array,
        ArrayGroupBy => ChannelType::Map,
        ArraySortBy | ArrayUnique | ArrayChunk | ArrayWindow => ChannelType::Array,
        ArrayPartition => ChannelType::Array,

        // Map Higher-Order
        MapMapValues | MapFilterEntries => ChannelType::Map,

        StringChars => ChannelType::Array,

        // Math Aggregate
        MathSum | MathProduct | MathMinOf | MathMaxOf => ChannelType::Null,
        MathAverage => ChannelType::Float64,
        MathCount => ChannelType::Int64,

        // Type Checking → bool
        IsNull | IsString | IsNumber | IsArray | IsMap | IsBool | IsBytes => ChannelType::Bool,

        // Math Extended
        Sin | Cos | Tan | Asin | Acos | Atan | Sinh | Cosh | Tanh | Ln | Log2 | Log10 | Exp
        | ToRadians | ToDegrees => ChannelType::Float64,
        Sign => ChannelType::Null,
        Log | Atan2 | Lerp | Remap => ChannelType::Float64,
        Clamp => ChannelType::Null,
        Gcd | Lcm => ChannelType::Int64,
        IsNan | IsInfinite | IsFinite | ApproxEq => ChannelType::Bool,

        RandomInt => ChannelType::Int64,
        RandomFloat => ChannelType::Float64,
        RandomBool => ChannelType::Bool,
        RandomBytes => ChannelType::Bytes,
        RandomRange => ChannelType::Null,
        RandomChoice => ChannelType::Null,
        RandomShuffle | RandomSample => ChannelType::Array,
        RandomUuid | RandomString => ChannelType::String,

        FsRead => ChannelType::String,
        FsWrite | FsAppend => ChannelType::Bool,
        FsExists | FsIsFile | FsIsDir => ChannelType::Bool,
        FsList => ChannelType::Array,
        FsMkdir | FsRemove | FsChmod | FsSymlink => ChannelType::Bool,
        FsCopy => ChannelType::Int64,
        FsMove => ChannelType::Bool,
        FsSize => ChannelType::Int64,
        FsReadlink => ChannelType::String,

        EnvGet => ChannelType::Null,
        EnvHas => ChannelType::Bool,
        EnvKeys => ChannelType::Array,
        OsName | OsArch | CurrentDir => ChannelType::String,
        ProcessPid => ChannelType::Int64,

        HttpGet | HttpPost | HttpPut | HttpDelete | HttpPatch => ChannelType::String,
        HttpRequest | HttpHead | HttpOptions => ChannelType::Map,
        UrlParse => ChannelType::Map,
        UrlJoin => ChannelType::String,

        // TCP
        TcpConnect => ChannelType::String,
        TcpWrite => ChannelType::Int64,
        TcpRead => ChannelType::Bytes,
        TcpClose | TcpServerClose => ChannelType::Null,
        TcpBind => ChannelType::String,
        TcpAccept => ChannelType::Map,

        // UDP
        UdpBind => ChannelType::String,
        UdpSendTo => ChannelType::Int64,
        UdpRecvFrom => ChannelType::Map,
        UdpClose => ChannelType::Null,

        // WebSocket
        WsConnect => ChannelType::String,
        WsSend | WsClose => ChannelType::Null,
        WsReceive => ChannelType::String,

        // SSE
        SseConnect => ChannelType::String,
        SseReadEvent => ChannelType::Map,
        SseClose => ChannelType::Null,

        // HTTP Server
        HttpServerStart => ChannelType::String,
        HttpServerReceive => ChannelType::Map,
        HttpServerRespond | HttpServerStop => ChannelType::Null,

        CertGenerate | CertParse | CertInfo | CertVerify | KeyGenerate | CertSelfSigned => {
            ChannelType::Map
        }

        PathJoin | PathBasename | PathDirname | PathExtension | PathStem | PathNormalize
        | PathWithExtension | PathParent => ChannelType::String,
        PathIsAbsolute => ChannelType::Bool,
        PathSplit => ChannelType::Array,

        // YAML/TOML
        YamlParse | TomlParse => ChannelType::Null,
        YamlStringify | TomlStringify => ChannelType::String,
        YamlValidate => ChannelType::Bool,
        YamlToJson | YamlFromJson | YamlMerge | XmlParse | XmlStringify => ChannelType::String,

        // CSV
        CsvParse => ChannelType::Array,
        CsvStringify => ChannelType::String,
        CsvHeaders => ChannelType::Array,
        CsvParseRows => ChannelType::Array,

        // Regex Extended
        RegexSplit | RegexFindAll | RegexCaptures => ChannelType::Array,
        RegexTest => ChannelType::Bool,
        RegexEscape => ChannelType::String,

        // UUID
        UuidV4 => ChannelType::String,
        UuidParse => ChannelType::Map,
        UuidIsValid => ChannelType::Bool,
        UuidNil => ChannelType::String,

        // Crypto Extended
        HashSha512 => ChannelType::String,
        HashCrc32 => ChannelType::Int64,
        HmacSha256 => ChannelType::String,
        ConstantTimeEq => ChannelType::Bool,

        CompressZstd | CompressLz4 | CompressGzip => ChannelType::Bytes,
        DecompressZstd | DecompressLz4 | DecompressGzip => ChannelType::Bytes,

        FmtNumber | FmtBytes | FmtDuration | FmtHex | FmtBinary | FmtPercent => ChannelType::String,

        // Convert Extended
        ParseInt => ChannelType::Int64,
        ParseFloat => ChannelType::Float64,

        // Time Extended
        Duration | AddDuration | SubDuration | Elapsed => ChannelType::Int64,
        TimeSleep => ChannelType::Null,
        TimeDiff => ChannelType::Int64,
        StartOf | EndOf => ChannelType::Int64,

        StatsMean | StatsMedian | StatsVariance | StatsStdDev | StatsPercentile | StatsQuantile
        | StatsCovariance | StatsCorrelation => ChannelType::Float64,
        StatsMode | StatsSum => ChannelType::Null,
        StatsMinBy | StatsMaxBy => ChannelType::Null,

        TextWrap | TextDedent | TextIndent | TextPadLeft | TextPadRight | TextTruncate
        | TextSlug | TextCamelCase | TextSnakeCase | TextTitleCase => ChannelType::String,

        HtmlEscape | HtmlUnescape | Base32Encode => ChannelType::String,
        Base32Decode => ChannelType::Bytes,

        ReflectTypeOf | ReflectTypeName | ReflectInspect => ChannelType::String,
        ReflectIsType => ChannelType::Bool,
        ReflectFields => ChannelType::Array,
        ReflectHasField | ReflectCallable => ChannelType::Bool,
        ReflectArity => ChannelType::Int64,

        SetFrom => ChannelType::Array,
        SetUnion | SetIntersection | SetDifference | SetSymmetricDifference => ChannelType::Array,
        Counter | OrderedMap => ChannelType::Map,
        MostCommon => ChannelType::Array,

        SortAsc | SortDesc | StableSort | SortReverse | SortBy | SortByKey => ChannelType::Array,
        IsSorted => ChannelType::Bool,
        BinarySearch => ChannelType::Int64,

        Exec => ChannelType::Map,
        ExecStatus => ChannelType::Int64,
        ExecOutput => ChannelType::String,

        MutexNew => ChannelType::String,
        MutexLock | MutexUnlock => ChannelType::Null,
        WaitgroupNew => ChannelType::String,
        WaitgroupDone | WaitgroupWait => ChannelType::Null,

        AwaitAll => ChannelType::Array,

        LogInfo | LogWarn | LogError | LogDebug => ChannelType::Null,

        IterChain | IterCycle | IterRepeat | IterProduct | IterPairwise => ChannelType::Array,

        TemplateRender => ChannelType::String,

        FlagParse => ChannelType::Map,
        FlagArgs => ChannelType::Array,

        MathGamma | MathLgamma | MathErf | MathErfc | MathExpm1 | MathNextafter | MathSignbit => ChannelType::Null,
        FsChown | FsHardlink | OsPipe => ChannelType::Null,
        FormatFloat => ChannelType::String,
        ParseUint => ChannelType::Null,
        CliFix | CliClean | CliTree => ChannelType::Null,
        // Platform — Terminal
        RawModeEnable | RawModeDisable => ChannelType::Null,
        ReadByte => ChannelType::Null,
        ReadByteTimeout => ChannelType::Null,
        // Platform — SDL2
        SdlInit => ChannelType::Null,
        SdlSetColor | SdlClear | SdlPresent | SdlDrawPixel | SdlDrawLine | SdlFillRect => ChannelType::Null,
        SdlPollEvent => ChannelType::Null,
        SdlDelay => ChannelType::Null,
        SdlTicks => ChannelType::Null,
        SdlDestroy => ChannelType::Null,
        // Platform — Audio
        AudioStreamNew => ChannelType::Null,
        AudioWriteSamples => ChannelType::Null,
        AudioDrain => ChannelType::Null,
        AudioClose => ChannelType::Null,
        // Platform — WebGPU
        GpuInit => ChannelType::Null,
        GpuCreateBuffer | GpuCreateShader | GpuCreatePipeline | GpuCreateTexture => ChannelType::Null,
        GpuBeginRenderPass | GpuDraw | GpuEndRenderPass | GpuSubmit | GpuPresent => ChannelType::Null,
        GpuWriteBuffer => ChannelType::Null,
        GpuDestroy => ChannelType::Null,
    }
}

/// Returns the expected input types for an operation: `Vec<(port_name, ChannelType)>`.
/// `ChannelType::Null` means "any type" (polymorphic port).
pub fn op_input_types(op: OperationType) -> &'static [(&'static str, ChannelType)] {
    use ChannelType::*;
    use OperationType::*;
    match op {
        // Arithmetic: numeric inputs
        Add | Subtract | Multiply | Divide | Modulo | Power | Min | Max => {
            &[("a", Null), ("b", Null)]
        }
        Sqrt | Cbrt | Negate | Abs | Round | Floor | Ceil => &[("value", Null)],
        Hypot => &[("a", Null), ("b", Null)],

        // Comparison: numeric or polymorphic
        Greater | Less | GreaterEq | LessEq => &[("a", Null), ("b", Null)],
        Equal | NotEqual => &[("a", Null), ("b", Null)],

        // Logic: bool inputs
        And | Or | Xor => &[("a", Bool), ("b", Bool)],
        Not => &[("value", Bool)],

        // Bitwise: numeric inputs
        BitAnd | BitOr | BitXor | BitShiftLeft | BitShiftRight => {
            &[("a", Null), ("b", Null)]
        }
        BitNot => &[("value", Null)],

        // String ops: string inputs
        Concat => &[("a", String), ("b", String)],
        Split => &[("input", String), ("delimiter", String)],
        Replace => &[("input", String), ("search", String), ("replace", String)],
        Contains => &[("input", String), ("search", String)],
        StartsWith => &[("input", String), ("prefix", String)],
        EndsWith => &[("input", String), ("suffix", String)],
        IndexOf | StringCount => &[("input", String), ("search", String)],
        RegexReplace => &[("input", String), ("replacement", String), ("pattern", String)],
        Substring | Length | ToUpper | ToLower | Trim | TrimStart | TrimEnd | CharAt | PadStart
        | PadEnd | StringReverse | StringRepeat | StringLines | StringWords => {
            &[("input", String)]
        }
        RegexMatch | RegexExtract => &[("input", String), ("pattern", String)],
        StringFormat => &[("template", String), ("values", Map)],
        StringJoin => &[("array", Array)],
        StringTemplate => &[("template", String), ("values", Array)],

        IfElse => &[("condition", Bool), ("then", Null), ("else", Null)],
        Switch => &[("value", Null), ("default", Null)],
        Coalesce => &[("a", Null), ("b", Null)],
        Assert => &[("condition", Bool), ("message", String)],
        DebugLog => &[("input", Null)],
        TryCatch => &[("input", Null), ("fallback", Null)],
        Error => &[("message", String)],

        // V2 Language Constructs
        FunctionDef | FunctionCall => &[],
        AsyncSpawn | AsyncAwait => &[("input", Null)],
        LoopGroup => &[],

        // Type conversion — accepts anything
        ToString | ToInt64 | ToFloat64 | ToBool | ToBytes | ToJson | Typeof
        | CharFromCode | CharCode => {
            &[("input", Null)]
        }
        FromBytes => &[("input", Bytes)],
        ParseJson => &[("input", String)],
        Default => &[("input", Null), ("fallback", Null)],

        ArrayGet => &[("array", Array), ("index", Null)],
        ArraySet => &[("array", Array), ("index", Null), ("value", Null)],
        ArrayPush => &[("array", Array), ("value", Null)],
        ArrayLength => &[("array", Array)],
        ArraySlice => &[("array", Array)],
        ArrayConcat => &[("a", Array), ("b", Array)],
        ArrayContains => &[("array", Array), ("value", Null)],
        ArrayReverse | ArrayFlatten | ArraySort | ArrayFilterNulls | ArrayPop | ArrayShift => {
            &[("array", Array)]
        }
        ArrayInsert => &[("array", Array), ("index", Null), ("value", Null)],
        ArrayRemove => &[("array", Array), ("index", Null)],
        ArrayFromMap => &[("map", Map)],
        ArrayJoin => &[("array", Array)],
        Range => &[("start", Null), ("end", Null)],
        Reduce => &[("array", Array), ("initial", Null)],

        MapGet => &[("map", Map), ("key", String)],
        MapSet => &[("map", Map), ("key", String), ("value", Null)],
        MapDelete => &[("map", Map), ("key", String)],
        MapHas => &[("map", Map), ("key", String)],
        MapKeys | MapValues | MapEntries | MapSize => &[("map", Map)],
        MapMerge => &[("a", Map), ("b", Map)],
        MapFromEntries => &[("array", Array)],
        MapUpdate => &[("map", Map), ("key", String), ("value", Null)],

        BytesLength | BytesSlice | BytesTrim => &[("input", Bytes)],
        BytesConcat => &[("a", Bytes), ("b", Bytes)],
        BytesContains => &[("input", Bytes), ("search", Bytes)],
        BytesCompare | BytesEqual => &[("a", Bytes), ("b", Bytes)],
        BytesHasPrefix | BytesHasSuffix => &[("input", Bytes), ("prefix", Bytes)],
        BytesIndex => &[("input", Bytes), ("needle", Bytes)],
        BytesJoin => &[("array", Array), ("separator", Bytes)],
        BytesRepeat => &[("input", Bytes), ("count", Int64)],
        BytesSplit => &[("input", Bytes), ("separator", Bytes)],
        BytesFromString => &[("input", String)],
        BytesToString => &[("input", Bytes)],
        Base64Encode => &[("input", Bytes)],
        Base64Decode => &[("input", String)],

        ErrorNew => &[("message", String)],
        ErrorWrap => &[("inner", Map), ("message", String)],
        ErrorUnwrap | ErrorChain => &[("error", Map)],
        ErrorIs => &[("error", Map), ("target", String)],

        // JSON
        JsonGet => &[("value", Null), ("path", String)],
        JsonSet => &[("value", Null), ("path", String), ("item", Null)],
        JsonDelete => &[("value", Null), ("path", String)],
        JsonFlatten => &[("input", Null)],
        JsonMerge => &[("a", Null), ("b", Null)],
        JsonType | JsonValidate | JsonPrettyPrint | JsonCompact => &[("input", Null)],
        JsonQuery => &[("value", Null), ("path", String)],

        // DateTime
        NowTimestamp => &[],
        FormatTimestamp | ParseTimestamp => &[("input", Null)],
        TimestampAdd => &[("input", Null), ("amount", Null)],
        TimestampDiff => &[("a", Null), ("b", Null)],
        Sleep => &[("duration", Null)],

        // Hash/Encode
        HashSha256 | HashSha1 | HashBlake3 | HashMd5 | UrlEncode | UrlDecode | HexDecode => {
            &[("input", String)]
        }
        HexEncode => &[("input", Bytes)],

        // Array Higher-Order (HOFs taking array + config operation)
        ArrayMap | ArrayFilter | ArrayFlatMap | ArrayFind | ArrayFindIndex | ArrayEvery
        | ArraySome | ArrayTakeWhile | ArraySkipWhile | ArrayGroupBy | ArraySortBy
        | ArrayPartition => &[("array", Array)],
        ArrayScan => &[("array", Array), ("initial", Null)],
        ArrayZip => &[("a", Array), ("b", Array)],
        ArrayEnumerate | ArrayUnique => &[("array", Array)],
        ArrayTake | ArraySkip | ArrayChunk | ArrayWindow => &[("array", Array)],

        // Map Higher-Order
        MapMapValues | MapFilterEntries => &[("map", Map)],

        StringChars => &[("input", String)],

        // Math Aggregate
        MathSum | MathProduct | MathAverage | MathMinOf | MathMaxOf | MathCount => {
            &[("array", Array)]
        }

        // Type Checking
        IsNull | IsString | IsNumber | IsArray | IsMap | IsBool | IsBytes => &[("input", Null)],

        // Math Extended
        Sin | Cos | Tan | Asin | Acos | Atan | Sinh | Cosh | Tanh | Ln | Log2 | Log10 | Exp
        | ToRadians | ToDegrees | Sign | IsNan | IsInfinite | IsFinite => &[("value", Null)],
        Log => &[("value", Null), ("base", Null)],
        Atan2 | Gcd | Lcm => &[("a", Null), ("b", Null)],
        ApproxEq => &[("a", Null), ("b", Null), ("epsilon", Null)],
        Clamp => &[("value", Null), ("min", Null), ("max", Null)],
        Lerp => &[("a", Null), ("b", Null), ("t", Null)],
        Remap => &[
            ("value", Null),
            ("in_min", Null),
            ("in_max", Null),
            ("out_min", Null),
            ("out_max", Null),
        ],

        RandomInt | RandomFloat | RandomBool | RandomBytes | RandomUuid | RandomString => &[],
        RandomRange => &[("a", Null), ("b", Null)],
        RandomChoice | RandomShuffle | RandomSample => &[("array", Array)],

        FsRead | FsExists | FsList | FsMkdir | FsSize | FsIsFile | FsIsDir | FsRemove => {
            &[("path", String)]
        }
        FsWrite | FsAppend => &[("path", String), ("content", Null)],
        FsCopy | FsMove => &[("source", String), ("destination", String)],
        FsChmod => &[("path", String), ("mode", Null)],
        FsSymlink => &[("target", String), ("link", String)],
        FsReadlink => &[("path", String)],

        EnvGet | EnvHas => &[("key", String)],
        EnvKeys | OsName | OsArch | ProcessPid | CurrentDir => &[],

        HttpGet | HttpDelete | HttpHead | HttpOptions => &[("url", String)],
        HttpPost | HttpPut | HttpPatch => &[("url", String), ("body", Null)],
        HttpRequest => &[
            ("method", String),
            ("url", String),
            ("body", Null),
            ("headers", Map),
        ],
        UrlParse => &[("input", String)],
        UrlJoin => &[("base", String), ("path", String)],

        // TCP
        TcpConnect => &[("host", String), ("port", Null)],
        TcpWrite => &[("conn_id", String), ("data", Null)],
        TcpRead => &[("conn_id", String)],
        TcpClose => &[("conn_id", String)],
        TcpBind => &[("address", String), ("port", Null)],
        TcpAccept => &[("listener_id", String)],
        TcpServerClose => &[("listener_id", String)],

        // UDP
        UdpBind => &[("address", String), ("port", Null)],
        UdpSendTo => &[
            ("socket_id", String),
            ("data", Null),
            ("address", String),
            ("port", Null),
        ],
        UdpRecvFrom => &[("socket_id", String)],
        UdpClose => &[("socket_id", String)],

        // WebSocket
        WsConnect => &[("url", String)],
        WsSend => &[("conn_id", String), ("message", Null)],
        WsReceive => &[("conn_id", String)],
        WsClose => &[("conn_id", String)],

        // SSE
        SseConnect => &[("url", String)],
        SseReadEvent => &[("conn_id", String)],
        SseClose => &[("conn_id", String)],

        // HTTP Server
        HttpServerStart => &[("address", String), ("port", Null)],
        HttpServerReceive => &[("server_id", String)],
        HttpServerRespond => &[("client_id", String), ("status", Null), ("body", Null)],
        HttpServerStop => &[("server_id", String)],

        CertGenerate | CertSelfSigned => &[("cn", String)],
        CertParse | CertInfo => &[("pem", String)],
        CertVerify => &[("pem", String)],
        KeyGenerate => &[],

        PathJoin => &[("a", String), ("b", String)],
        PathBasename | PathDirname | PathExtension | PathStem | PathIsAbsolute | PathNormalize
        | PathSplit | PathParent => &[("input", String)],
        PathWithExtension => &[("input", String), ("extension", String)],

        // YAML/TOML
        YamlParse | YamlStringify | YamlValidate | YamlToJson | YamlFromJson | TomlParse
        | TomlStringify => &[("input", Null)],
        YamlMerge => &[("a", String), ("b", String)],
        XmlParse | XmlStringify => &[("input", Null)],

        // CSV
        CsvParse | CsvStringify | CsvHeaders | CsvParseRows => &[("input", Null)],

        // Regex Extended
        RegexSplit | RegexTest | RegexCaptures | RegexFindAll => {
            &[("input", String), ("pattern", String)]
        }
        RegexEscape => &[("input", String)],

        // UUID
        UuidV4 | UuidNil => &[],
        UuidParse | UuidIsValid => &[("input", String)],

        // Crypto Extended
        HashSha512 | HashCrc32 => &[("input", String)],
        HmacSha256 => &[("input", String), ("key", String)],
        ConstantTimeEq => &[("a", Null), ("b", Null)],

        CompressZstd | DecompressZstd | CompressLz4 | DecompressLz4 | CompressGzip | DecompressGzip => &[("input", Null)],

        FmtNumber | FmtBytes | FmtDuration | FmtHex | FmtBinary | FmtPercent => {
            &[("value", Null)]
        }

        // Convert Extended
        ParseInt | ParseFloat => &[("input", String)],

        // Time Extended
        Duration => &[],
        Elapsed => &[("timestamp", Null)],
        TimeSleep => &[("duration", Null)],
        AddDuration | SubDuration => &[("timestamp", Null), ("duration", Null)],
        TimeDiff => &[("a", Null), ("b", Null)],
        StartOf | EndOf => &[("input", Null)],

        StatsMean | StatsMedian | StatsMode | StatsVariance | StatsStdDev | StatsSum => {
            &[("array", Array)]
        }
        StatsMinBy | StatsMaxBy => &[("array", Array), ("key", String)],
        StatsPercentile => &[("array", Array), ("percentile", Null)],
        StatsQuantile => &[("array", Array), ("quantile", Null)],
        StatsCovariance | StatsCorrelation => &[("a", Array), ("b", Array)],

        TextWrap | TextDedent | TextIndent | TextPadLeft | TextPadRight | TextTruncate
        | TextSlug | TextCamelCase | TextSnakeCase | TextTitleCase => &[("input", String)],

        HtmlEscape | HtmlUnescape | Base32Encode | Base32Decode => &[("input", Null)],

        ReflectTypeOf | ReflectTypeName | ReflectFields | ReflectCallable | ReflectArity
        | ReflectInspect => &[("input", Null)],
        ReflectIsType => &[("input", Null), ("type_name", String)],
        ReflectHasField => &[("input", Null), ("field", String)],

        SetFrom | Counter | OrderedMap => &[("array", Array)],
        SetUnion | SetIntersection | SetDifference | SetSymmetricDifference => {
            &[("a", Array), ("b", Array)]
        }
        MostCommon => &[("array", Array)],

        SortAsc | SortDesc | StableSort | IsSorted | SortReverse | SortBy | SortByKey => {
            &[("array", Array)]
        }
        BinarySearch => &[("array", Array), ("value", Null)],

        Exec | ExecStatus | ExecOutput => &[("command", String)],

        MutexNew => &[],
        MutexLock | MutexUnlock => &[("id", String)],
        WaitgroupNew => &[("count", Null)],
        WaitgroupDone | WaitgroupWait => &[("id", String)],

        AwaitAll => &[("futures", Array)],

        LogInfo | LogWarn | LogError | LogDebug => &[("message", Null)],

        IterChain => &[("array", Array), ("other", Array)],
        IterCycle => &[("array", Array), ("count", Null)],
        IterRepeat => &[("value", Null), ("count", Null)],
        IterProduct => &[("array", Array), ("other", Array)],
        IterPairwise => &[("array", Array)],

        TemplateRender => &[("template", String), ("data", Map)],

        FlagParse => &[("args", Array), ("spec", Map)],
        FlagArgs => &[],

        MathGamma | MathLgamma | MathErf | MathErfc | MathExpm1 | MathSignbit => &[("value", Null)],
        MathNextafter => &[("x", Null), ("y", Null)],
        FsChown => &[("path", String), ("uid", Null), ("gid", Null)],
        FsHardlink => &[("src", String), ("dst", String)],
        OsPipe => &[],
        FormatFloat => &[("value", Null), ("format", String)],
        ParseUint => &[("value", String), ("base", Null)],
        CliFix | CliClean | CliTree => &[],
        // Platform — Terminal
        RawModeEnable | RawModeDisable => &[],
        ReadByte => &[],
        ReadByteTimeout => &[("deciseconds", Null)],
        // Platform — SDL2
        SdlInit => &[("title", String), ("width", Null), ("height", Null)],
        SdlSetColor => &[("handle", Null), ("r", Null), ("g", Null), ("b", Null)],
        SdlClear | SdlPresent | SdlDestroy => &[("handle", Null)],
        SdlDrawPixel => &[("handle", Null), ("x", Null), ("y", Null)],
        SdlDrawLine => &[("handle", Null), ("x1", Null), ("y1", Null), ("x2", Null), ("y2", Null)],
        SdlFillRect => &[("handle", Null), ("x", Null), ("y", Null), ("w", Null), ("h", Null)],
        SdlPollEvent => &[("handle", Null)],
        SdlDelay => &[("ms", Null)],
        SdlTicks => &[("handle", Null)],
        // Platform — Audio
        AudioStreamNew => &[("sample_rate", Null)],
        AudioWriteSamples => &[("handle", Null), ("samples", Array)],
        AudioDrain | AudioClose => &[("handle", Null)],
        // Platform — WebGPU
        GpuInit => &[("canvas", Null)],
        GpuCreateBuffer => &[("device", Null), ("size", Null), ("usage", Null)],
        GpuCreateShader => &[("device", Null), ("code", String)],
        GpuCreatePipeline => &[("device", Null), ("vertex", Null), ("fragment", Null)],
        GpuBeginRenderPass => &[("encoder", Null), ("descriptor", Null)],
        GpuDraw => &[("pass", Null), ("vertex_count", Null)],
        GpuEndRenderPass => &[("pass", Null)],
        GpuSubmit => &[("device", Null), ("commands", Null)],
        GpuPresent => &[("device", Null)],
        GpuWriteBuffer => &[("device", Null), ("buffer", Null), ("data", Array)],
        GpuCreateTexture => &[("device", Null), ("width", Null), ("height", Null), ("format", Null)],
        GpuDestroy => &[("device", Null)],
    }
}

/// Returns the input port names for an operation.
/// Returns a static slice to avoid heap allocation on every call.
pub fn op_input_ports(op: OperationType) -> &'static [&'static str] {
    use OperationType::*;
    match op {
        // Binary: a, b
        Add | Subtract | Multiply | Divide | Modulo | Power | Min | Max | Equal | NotEqual
        | Greater | Less | GreaterEq | LessEq | And | Or | Xor | BitAnd | BitOr | BitXor
        | BitShiftLeft | BitShiftRight | Concat => &["a", "b"],

        // Unary: value (arithmetic/logic)
        Sqrt | Cbrt | Negate | Abs | Round | Floor | Ceil | Not | BitNot => &["value"],
        Hypot => &["a", "b"],

        // Unary: input (string + type conversion)
        Substring | Length | ToUpper | ToLower | Trim | TrimStart | TrimEnd | CharAt | PadStart
        | PadEnd | StringReverse | StringRepeat | StringLines | StringWords
        | ToString | ToInt64 | ToFloat64 | ToBool | ToBytes | FromBytes
        | ParseJson | ToJson | CharFromCode | CharCode => &["input"],

        // String ops with specific ports
        Split => &["input", "delimiter"],
        Replace => &["input", "search", "replace"],
        Contains => &["input", "search"],
        StartsWith => &["input", "prefix"],
        EndsWith => &["input", "suffix"],
        IndexOf | StringCount => &["input", "search"],
        RegexMatch | RegexExtract => &["input", "pattern"],
        RegexReplace => &["input", "replacement", "pattern"],
        StringJoin => &["array"],
        StringTemplate => &["template", "values"],

        IfElse => &["condition", "then", "else"],
        Switch => &["value", "default"],
        Coalesce => &["a", "b"],
        TryCatch => &["input", "fallback"],
        Error => &["message"],

        ArrayGet => &["array", "index"],
        ArraySet => &["array", "index", "value"],
        ArrayPush => &["array", "value"],
        ArrayLength => &["array"],
        ArraySlice => &["array"],
        ArrayConcat => &["a", "b"],
        ArrayContains => &["array", "value"],
        ArrayReverse | ArrayFlatten | ArraySort | ArrayFilterNulls | ArrayPop | ArrayShift => {
            &["array"]
        }
        ArrayInsert => &["array", "index", "value"],
        ArrayRemove => &["array", "index"],
        ArrayFromMap => &["map"],
        ArrayJoin => &["array"],

        MapGet => &["map", "key"],
        MapSet => &["map", "key", "value"],
        MapDelete => &["map", "key"],
        MapHas => &["map", "key"],
        MapKeys | MapValues | MapEntries | MapSize => &["map"],
        MapMerge => &["a", "b"],
        MapFromEntries => &["array"],
        MapUpdate => &["map", "key", "value"],

        BytesLength | BytesSlice | BytesTrim => &["input"],
        BytesConcat => &["a", "b"],
        BytesContains => &["input", "search"],
        BytesCompare | BytesEqual => &["a", "b"],
        BytesHasPrefix | BytesHasSuffix => &["input", "prefix"],
        BytesIndex => &["input", "needle"],
        BytesJoin => &["array", "separator"],
        BytesRepeat => &["input", "count"],
        BytesSplit => &["input", "separator"],
        BytesFromString | BytesToString => &["input"],
        Base64Encode | Base64Decode => &["input"],

        ErrorNew => &["message"],
        ErrorWrap => &["inner", "message"],
        ErrorUnwrap | ErrorChain => &["error"],
        ErrorIs => &["error", "target"],

        Range => &["start", "end"],
        Reduce => &["array", "initial"],

        // JSON
        JsonGet => &["value", "path"],
        JsonSet => &["value", "path", "item"],
        JsonDelete => &["value", "path"],
        JsonFlatten => &["input"],
        JsonMerge => &["a", "b"],
        JsonType | JsonValidate | JsonPrettyPrint | JsonCompact => &["input"],
        JsonQuery => &["value", "path"],

        // DateTime
        NowTimestamp => &[],
        FormatTimestamp => &["input"],
        ParseTimestamp => &["input"],
        TimestampAdd => &["input", "amount"],
        TimestampDiff => &["a", "b"],
        Sleep => &["duration"],

        // Hash/Encode
        HashSha256 | HashSha1 | HashBlake3 | HashMd5 | UrlEncode | UrlDecode | HexDecode => &["input"],
        HexEncode => &["input"],

        StringFormat => &["template", "values"],

        Assert => &["condition", "message"],
        DebugLog => &["input"],

        Typeof => &["input"],
        Default => &["input", "fallback"],

        // Array Higher-Order
        ArrayMap | ArrayFilter | ArrayFlatMap | ArrayFind | ArrayFindIndex | ArrayEvery
        | ArraySome | ArrayTakeWhile | ArraySkipWhile | ArrayGroupBy | ArraySortBy
        | ArrayPartition => &["array"],
        ArrayScan => &["array", "initial"],
        ArrayZip => &["a", "b"],
        ArrayEnumerate | ArrayUnique => &["array"],
        ArrayTake | ArraySkip | ArrayChunk | ArrayWindow => &["array"],

        // Map Higher-Order
        MapMapValues | MapFilterEntries => &["map"],

        StringChars => &["input"],

        // Math Aggregate
        MathSum | MathProduct | MathAverage | MathMinOf | MathMaxOf | MathCount => &["array"],

        // Type Checking
        IsNull | IsString | IsNumber | IsArray | IsMap | IsBool | IsBytes => &["input"],

        // Math Extended
        Sin | Cos | Tan | Asin | Acos | Atan | Sinh | Cosh | Tanh | Ln | Log2 | Log10 | Exp
        | ToRadians | ToDegrees | Sign | IsNan | IsInfinite | IsFinite => &["value"],
        Log => &["value", "base"],
        Atan2 | Gcd | Lcm => &["a", "b"],
        ApproxEq => &["a", "b", "epsilon"],
        Clamp => &["value", "min", "max"],
        Lerp => &["a", "b", "t"],
        Remap => &["value", "in_min", "in_max", "out_min", "out_max"],

        RandomInt | RandomFloat | RandomBool | RandomBytes | RandomUuid | RandomString => &[],
        RandomRange => &["a", "b"],
        RandomChoice | RandomShuffle | RandomSample => &["array"],

        FsRead | FsExists | FsList | FsMkdir | FsSize | FsIsFile | FsIsDir | FsRemove => {
            &["path"]
        }
        FsWrite | FsAppend => &["path", "content"],
        FsCopy | FsMove => &["source", "destination"],
        FsChmod => &["path", "mode"],
        FsSymlink => &["target", "link"],
        FsReadlink => &["path"],

        EnvGet | EnvHas => &["key"],
        EnvKeys | OsName | OsArch | ProcessPid | CurrentDir => &[],

        HttpGet | HttpDelete | HttpHead | HttpOptions => &["url"],
        HttpPost | HttpPut | HttpPatch => &["url", "body"],
        HttpRequest => &["method", "url", "body", "headers"],
        UrlParse => &["input"],
        UrlJoin => &["base", "path"],

        // TCP
        TcpConnect => &["host", "port"],
        TcpWrite => &["conn_id", "data"],
        TcpRead => &["conn_id"],
        TcpClose => &["conn_id"],
        TcpBind => &["address", "port"],
        TcpAccept => &["listener_id"],
        TcpServerClose => &["listener_id"],

        // UDP
        UdpBind => &["address", "port"],
        UdpSendTo => &["socket_id", "data", "address", "port"],
        UdpRecvFrom => &["socket_id"],
        UdpClose => &["socket_id"],

        // WebSocket
        WsConnect => &["url"],
        WsSend => &["conn_id", "message"],
        WsReceive | WsClose => &["conn_id"],

        // SSE
        SseConnect => &["url"],
        SseReadEvent | SseClose => &["conn_id"],

        // HTTP Server
        HttpServerStart => &["address", "port"],
        HttpServerReceive => &["server_id"],
        HttpServerRespond => &["client_id", "status", "body"],
        HttpServerStop => &["server_id"],

        CertGenerate | CertSelfSigned => &["cn"],
        CertParse | CertInfo | CertVerify => &["pem"],
        KeyGenerate => &[],

        PathJoin => &["a", "b"],
        PathBasename | PathDirname | PathExtension | PathStem | PathIsAbsolute | PathNormalize
        | PathSplit | PathParent => &["input"],
        PathWithExtension => &["input", "extension"],

        // YAML/TOML
        YamlParse | YamlStringify | YamlValidate | YamlToJson | YamlFromJson | TomlParse
        | TomlStringify => &["input"],
        YamlMerge => &["a", "b"],
        XmlParse | XmlStringify => &["input"],

        // CSV
        CsvParse | CsvStringify | CsvHeaders | CsvParseRows => &["input"],

        // Regex Extended
        RegexSplit | RegexTest | RegexCaptures | RegexFindAll => &["input", "pattern"],
        RegexEscape => &["input"],

        // UUID
        UuidV4 | UuidNil => &[],
        UuidParse | UuidIsValid => &["input"],

        // Crypto Extended
        HashSha512 | HashCrc32 => &["input"],
        HmacSha256 => &["input", "key"],
        ConstantTimeEq => &["a", "b"],

        CompressZstd | DecompressZstd | CompressLz4 | DecompressLz4 | CompressGzip | DecompressGzip => &["input"],

        FmtNumber | FmtBytes | FmtDuration | FmtHex | FmtBinary | FmtPercent => &["value"],

        // Convert Extended
        ParseInt | ParseFloat => &["input"],

        // Time Extended
        Duration => &[],
        Elapsed => &["timestamp"],
        TimeSleep => &["duration"],
        AddDuration | SubDuration => &["timestamp", "duration"],
        TimeDiff => &["a", "b"],
        StartOf | EndOf => &["input"],

        StatsMean | StatsMedian | StatsMode | StatsVariance | StatsStdDev | StatsSum => {
            &["array"]
        }
        StatsMinBy | StatsMaxBy => &["array", "key"],
        StatsPercentile => &["array", "percentile"],
        StatsQuantile => &["array", "quantile"],
        StatsCovariance | StatsCorrelation => &["a", "b"],

        TextWrap | TextDedent | TextIndent | TextPadLeft | TextPadRight | TextTruncate
        | TextSlug | TextCamelCase | TextSnakeCase | TextTitleCase => &["input"],

        // Encode Extended
        HtmlEscape | HtmlUnescape | Base32Encode | Base32Decode => &["input"],

        ReflectTypeOf | ReflectTypeName | ReflectFields | ReflectCallable | ReflectArity
        | ReflectInspect => &["input"],
        ReflectIsType => &["input", "type_name"],
        ReflectHasField => &["input", "field"],

        SetFrom | Counter | OrderedMap => &["array"],
        SetUnion | SetIntersection | SetDifference | SetSymmetricDifference => &["a", "b"],
        MostCommon => &["array"],

        SortAsc | SortDesc | StableSort | IsSorted | SortReverse | SortBy | SortByKey => {
            &["array"]
        }
        BinarySearch => &["array", "value"],

        // V2 Language Constructs
        FunctionDef | FunctionCall => &[],
        AsyncSpawn | AsyncAwait => &["input"],
        LoopGroup => &[],

        Exec | ExecStatus | ExecOutput => &["command"],

        MutexNew => &[],
        MutexLock | MutexUnlock => &["id"],
        WaitgroupNew => &["count"],
        WaitgroupDone | WaitgroupWait => &["id"],

        AwaitAll => &["futures"],

        LogInfo | LogWarn | LogError | LogDebug => &["message"],

        IterChain | IterProduct => &["array", "other"],
        IterCycle => &["array", "count"],
        IterRepeat => &["value", "count"],
        IterPairwise => &["array"],

        TemplateRender => &["template", "data"],

        FlagParse => &["args", "spec"],
        FlagArgs => &[],

        MathGamma | MathLgamma | MathErf | MathErfc | MathExpm1 | MathSignbit => &["value"],
        MathNextafter => &["x", "y"],
        FsChown => &["path", "uid", "gid"],
        FsHardlink => &["src", "dst"],
        OsPipe => &[],
        FormatFloat => &["value", "format"],
        ParseUint => &["value", "base"],
        CliFix | CliClean | CliTree => &[],
        // Platform — Terminal
        RawModeEnable | RawModeDisable => &[],
        ReadByte => &[],
        ReadByteTimeout => &["deciseconds"],
        // Platform — SDL2
        SdlInit => &["title", "width", "height"],
        SdlSetColor => &["handle", "r", "g", "b"],
        SdlClear | SdlPresent | SdlDestroy => &["handle"],
        SdlDrawPixel => &["handle", "x", "y"],
        SdlDrawLine => &["handle", "x1", "y1", "x2", "y2"],
        SdlFillRect => &["handle", "x", "y", "w", "h"],
        SdlPollEvent | SdlTicks => &["handle"],
        SdlDelay => &["ms"],
        // Platform — Audio
        AudioStreamNew => &["sample_rate"],
        AudioWriteSamples => &["handle", "samples"],
        AudioDrain | AudioClose => &["handle"],
        // Platform — WebGPU
        GpuInit => &["canvas"],
        GpuCreateBuffer => &["device", "size", "usage"],
        GpuCreateShader => &["device", "code"],
        GpuCreatePipeline => &["device", "vertex", "fragment"],
        GpuBeginRenderPass => &["encoder", "descriptor"],
        GpuDraw => &["pass", "vertex_count"],
        GpuEndRenderPass => &["pass"],
        GpuSubmit => &["device", "commands"],
        GpuPresent => &["device"],
        GpuWriteBuffer => &["device", "buffer", "data"],
        GpuCreateTexture => &["device", "width", "height", "format"],
        GpuDestroy => &["device"],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::OperationType;

    #[test]
    fn test_op_input_ports_and_types_same_length() {
        for &op in OperationType::ALL {
            let ports = op_input_ports(op);
            let types = op_input_types(op);
            assert_eq!(
                ports.len(),
                types.len(),
                "op_input_ports and op_input_types disagree on port count for {:?}: ports={:?}, types={:?}",
                op,
                ports,
                types.iter().map(|(n, _)| *n).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn test_op_input_ports_and_types_same_names() {
        for &op in OperationType::ALL {
            let ports = op_input_ports(op);
            let types = op_input_types(op);
            for (i, ((name, _), &port)) in types.iter().zip(ports.iter()).enumerate() {
                assert_eq!(
                    *name, port,
                    "op_input_ports and op_input_types disagree on port name #{} for {:?}: ports has {:?}, types has {:?}",
                    i, op, port, name
                );
            }
        }
    }

    #[test]
    fn test_op_output_type_all_variants() {
        // Ensures every variant produces a valid ChannelType (compiler guarantees exhaustive match)
        for &op in OperationType::ALL {
            let _ = op_output_type(op);
        }
    }

}
