//! Operation type mapping: output types and input port types.

use crate::types::{ChannelType, OperationType};

/// Returns the output `ChannelType` an operation produces.
pub fn op_output_type(op: OperationType) -> ChannelType {
    use OperationType::*;
    match op {
        // Arithmetic — output depends on inputs (Null port = accepts any type)
        Add | Subtract | Multiply | Modulo | Power | Sqrt | Negate | Abs | Min | Max | Round
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
        BytesSlice | BytesConcat => ChannelType::Bytes,
        BytesContains => ChannelType::Bool,
        Base64Encode | HexEncode => ChannelType::String,
        Base64Decode | HexDecode => ChannelType::Bytes,

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
        HashSha256 | HashBlake3 | HashMd5 | UrlEncode | UrlDecode => ChannelType::String,

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

        // String
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

        // Random
        RandomInt => ChannelType::Int64,
        RandomFloat => ChannelType::Float64,
        RandomBool => ChannelType::Bool,
        RandomBytes => ChannelType::Bytes,
        RandomRange => ChannelType::Null,
        RandomChoice => ChannelType::Null,
        RandomShuffle | RandomSample => ChannelType::Array,
        RandomUuid | RandomString => ChannelType::String,

        // Filesystem
        FsRead => ChannelType::String,
        FsWrite | FsAppend => ChannelType::Bool,
        FsExists | FsIsFile | FsIsDir => ChannelType::Bool,
        FsList => ChannelType::Array,
        FsMkdir | FsRemove => ChannelType::Bool,
        FsCopy => ChannelType::Int64,
        FsMove => ChannelType::Bool,
        FsSize => ChannelType::Int64,

        // Environment
        EnvGet => ChannelType::Null,
        EnvHas => ChannelType::Bool,
        EnvKeys => ChannelType::Array,
        OsName | OsArch | CurrentDir => ChannelType::String,
        ProcessPid => ChannelType::Int64,

        // Network
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

        // Certificate
        CertGenerate | CertParse | CertInfo | CertVerify | KeyGenerate | CertSelfSigned => {
            ChannelType::Map
        }

        // Path
        PathJoin | PathBasename | PathDirname | PathExtension | PathStem | PathNormalize
        | PathWithExtension | PathParent => ChannelType::String,
        PathIsAbsolute => ChannelType::Bool,
        PathSplit => ChannelType::Array,

        // YAML/TOML
        YamlParse | TomlParse => ChannelType::Null,
        YamlStringify | TomlStringify => ChannelType::String,
        YamlValidate => ChannelType::Bool,
        YamlToJson | YamlFromJson | YamlMerge => ChannelType::String,

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

        // Compress
        CompressZstd | CompressLz4 => ChannelType::Bytes,
        DecompressZstd | DecompressLz4 => ChannelType::Bytes,

        // Format
        FmtNumber | FmtBytes | FmtDuration | FmtHex | FmtBinary | FmtPercent => ChannelType::String,

        // Convert Extended
        ParseInt => ChannelType::Int64,
        ParseFloat => ChannelType::Float64,

        // Time Extended
        Duration | AddDuration | SubDuration | Elapsed => ChannelType::Int64,
        TimeSleep => ChannelType::Null,
        TimeDiff => ChannelType::Int64,
        StartOf | EndOf => ChannelType::Int64,

        // Stats
        StatsMean | StatsMedian | StatsVariance | StatsStdDev | StatsPercentile | StatsQuantile
        | StatsCovariance | StatsCorrelation => ChannelType::Float64,
        StatsMode | StatsSum => ChannelType::Null,
        StatsMinBy | StatsMaxBy => ChannelType::Null,

        // Text
        TextWrap | TextDedent | TextIndent | TextPadLeft | TextPadRight | TextTruncate
        | TextSlug | TextCamelCase | TextSnakeCase | TextTitleCase => ChannelType::String,

        // Encode
        HtmlEscape | HtmlUnescape | Base32Encode => ChannelType::String,
        Base32Decode => ChannelType::Bytes,

        // Reflect
        ReflectTypeOf | ReflectTypeName | ReflectInspect => ChannelType::String,
        ReflectIsType => ChannelType::Bool,
        ReflectFields => ChannelType::Array,
        ReflectHasField | ReflectCallable => ChannelType::Bool,
        ReflectArity => ChannelType::Int64,

        // Collections
        SetFrom => ChannelType::Array,
        SetUnion | SetIntersection | SetDifference | SetSymmetricDifference => ChannelType::Array,
        Counter | OrderedMap => ChannelType::Map,
        MostCommon => ChannelType::Array,

        // Sort
        SortAsc | SortDesc | StableSort | SortReverse | SortBy | SortByKey => ChannelType::Array,
        IsSorted => ChannelType::Bool,
        BinarySearch => ChannelType::Int64,
    }
}

/// Returns the expected input types for an operation: `Vec<(port_name, ChannelType)>`.
/// `ChannelType::Null` means "any type" (polymorphic port).
pub fn op_input_types(op: OperationType) -> Vec<(&'static str, ChannelType)> {
    use ChannelType::*;
    use OperationType::*;
    match op {
        // Arithmetic: numeric inputs
        Add | Subtract | Multiply | Divide | Modulo | Power | Min | Max => {
            vec![("a", Null), ("b", Null)]
        }
        Sqrt | Negate | Abs | Round | Floor | Ceil => vec![("value", Null)],

        // Comparison: numeric or polymorphic
        Greater | Less | GreaterEq | LessEq => vec![("a", Null), ("b", Null)],
        Equal | NotEqual => vec![("a", Null), ("b", Null)],

        // Logic: bool inputs
        And | Or | Xor => vec![("a", Bool), ("b", Bool)],
        Not => vec![("value", Bool)],

        // Bitwise: numeric inputs
        BitAnd | BitOr | BitXor | BitShiftLeft | BitShiftRight => {
            vec![("a", Null), ("b", Null)]
        }
        BitNot => vec![("value", Null)],

        // String ops: string inputs
        Concat => vec![("a", String), ("b", String)],
        Split => vec![("input", String), ("delimiter", String)],
        Replace => vec![("input", String), ("search", String), ("replace", String)],
        Contains => vec![("input", String), ("search", String)],
        StartsWith => vec![("input", String), ("prefix", String)],
        EndsWith => vec![("input", String), ("suffix", String)],
        IndexOf | StringCount => vec![("input", String), ("search", String)],
        RegexReplace => vec![("input", String), ("replacement", String), ("pattern", String)],
        Substring | Length | ToUpper | ToLower | Trim | TrimStart | TrimEnd | CharAt | PadStart
        | PadEnd | StringReverse | StringRepeat | StringLines | StringWords => {
            vec![("input", String)]
        }
        RegexMatch | RegexExtract => vec![("input", String), ("pattern", String)],
        StringFormat => vec![("template", String), ("values", Map)],
        StringJoin => vec![("array", Array)],
        StringTemplate => vec![("template", String), ("values", Array)],

        // Control flow
        IfElse => vec![("condition", Bool), ("then", Null), ("else", Null)],
        Switch => vec![("value", Null), ("default", Null)],
        Coalesce => vec![("a", Null), ("b", Null)],
        Assert => vec![("condition", Bool), ("message", String)],
        DebugLog => vec![("input", Null)],
        TryCatch => vec![("input", Null), ("fallback", Null)],
        Error => vec![("message", String)],

        // V2 Language Constructs
        FunctionDef | FunctionCall => vec![],
        AsyncSpawn | AsyncAwait => vec![("input", Null)],
        LoopGroup => vec![],

        // Type conversion — accepts anything
        ToString | ToInt64 | ToFloat64 | ToBool | ToBytes | ToJson | Typeof => {
            vec![("input", Null)]
        }
        FromBytes => vec![("input", Bytes)],
        ParseJson => vec![("input", String)],
        Default => vec![("input", Null), ("fallback", Null)],

        // Array ops
        ArrayGet => vec![("array", Array), ("index", Null)],
        ArraySet => vec![("array", Array), ("index", Null), ("value", Null)],
        ArrayPush => vec![("array", Array), ("value", Null)],
        ArrayLength => vec![("array", Array)],
        ArraySlice => vec![("array", Array)],
        ArrayConcat => vec![("a", Array), ("b", Array)],
        ArrayContains => vec![("array", Array), ("value", Null)],
        ArrayReverse | ArrayFlatten | ArraySort | ArrayFilterNulls | ArrayPop | ArrayShift => {
            vec![("array", Array)]
        }
        ArrayInsert => vec![("array", Array), ("index", Null), ("value", Null)],
        ArrayRemove => vec![("array", Array), ("index", Null)],
        ArrayFromMap => vec![("map", Map)],
        ArrayJoin => vec![("array", Array)],
        Range => vec![("start", Null), ("end", Null)],
        Reduce => vec![("array", Array), ("initial", Null)],

        // Map ops
        MapGet => vec![("map", Map), ("key", String)],
        MapSet => vec![("map", Map), ("key", String), ("value", Null)],
        MapDelete => vec![("map", Map), ("key", String)],
        MapHas => vec![("map", Map), ("key", String)],
        MapKeys | MapValues | MapEntries | MapSize => vec![("map", Map)],
        MapMerge => vec![("a", Map), ("b", Map)],
        MapFromEntries => vec![("array", Array)],
        MapUpdate => vec![("map", Map), ("key", String), ("value", Null)],

        // Bytes ops
        BytesLength | BytesSlice => vec![("input", Bytes)],
        BytesConcat => vec![("a", Bytes), ("b", Bytes)],
        BytesContains => vec![("input", Bytes), ("search", Bytes)],
        Base64Encode => vec![("input", Bytes)],
        Base64Decode => vec![("input", String)],

        // JSON
        JsonGet => vec![("value", Null), ("path", String)],
        JsonSet => vec![("value", Null), ("path", String), ("item", Null)],
        JsonDelete => vec![("value", Null), ("path", String)],
        JsonFlatten => vec![("input", Null)],
        JsonMerge => vec![("a", Null), ("b", Null)],
        JsonType | JsonValidate | JsonPrettyPrint | JsonCompact => vec![("input", Null)],
        JsonQuery => vec![("value", Null), ("path", String)],

        // DateTime
        NowTimestamp => vec![],
        FormatTimestamp | ParseTimestamp => vec![("input", Null)],
        TimestampAdd => vec![("input", Null), ("amount", Null)],
        TimestampDiff => vec![("a", Null), ("b", Null)],
        Sleep => vec![("duration", Null)],

        // Hash/Encode
        HashSha256 | HashBlake3 | HashMd5 | UrlEncode | UrlDecode | HexDecode => {
            vec![("input", String)]
        }
        HexEncode => vec![("input", Bytes)],

        // Array Higher-Order (HOFs taking array + config operation)
        ArrayMap | ArrayFilter | ArrayFlatMap | ArrayFind | ArrayFindIndex | ArrayEvery
        | ArraySome | ArrayTakeWhile | ArraySkipWhile | ArrayGroupBy | ArraySortBy
        | ArrayPartition => vec![("array", Array)],
        ArrayScan => vec![("array", Array), ("initial", Null)],
        ArrayZip => vec![("a", Array), ("b", Array)],
        ArrayEnumerate | ArrayUnique => vec![("array", Array)],
        ArrayTake | ArraySkip | ArrayChunk | ArrayWindow => vec![("array", Array)],

        // Map Higher-Order
        MapMapValues | MapFilterEntries => vec![("map", Map)],

        // String
        StringChars => vec![("input", String)],

        // Math Aggregate
        MathSum | MathProduct | MathAverage | MathMinOf | MathMaxOf | MathCount => {
            vec![("array", Array)]
        }

        // Type Checking
        IsNull | IsString | IsNumber | IsArray | IsMap | IsBool | IsBytes => vec![("input", Null)],

        // Math Extended
        Sin | Cos | Tan | Asin | Acos | Atan | Sinh | Cosh | Tanh | Ln | Log2 | Log10 | Exp
        | ToRadians | ToDegrees | Sign | IsNan | IsInfinite | IsFinite => vec![("value", Null)],
        Log => vec![("value", Null), ("base", Null)],
        Atan2 | Gcd | Lcm => vec![("a", Null), ("b", Null)],
        ApproxEq => vec![("a", Null), ("b", Null), ("epsilon", Null)],
        Clamp => vec![("value", Null), ("min", Null), ("max", Null)],
        Lerp => vec![("a", Null), ("b", Null), ("t", Null)],
        Remap => vec![
            ("value", Null),
            ("in_min", Null),
            ("in_max", Null),
            ("out_min", Null),
            ("out_max", Null),
        ],

        // Random
        RandomInt | RandomFloat | RandomBool | RandomBytes | RandomUuid | RandomString => vec![],
        RandomRange => vec![("a", Null), ("b", Null)],
        RandomChoice | RandomShuffle | RandomSample => vec![("array", Array)],

        // Filesystem
        FsRead | FsExists | FsList | FsMkdir | FsSize | FsIsFile | FsIsDir | FsRemove => {
            vec![("path", String)]
        }
        FsWrite | FsAppend => vec![("path", String), ("content", Null)],
        FsCopy | FsMove => vec![("source", String), ("destination", String)],

        // Environment
        EnvGet | EnvHas => vec![("key", String)],
        EnvKeys | OsName | OsArch | ProcessPid | CurrentDir => vec![],

        // Network
        HttpGet | HttpDelete | HttpHead | HttpOptions => vec![("url", String)],
        HttpPost | HttpPut | HttpPatch => vec![("url", String), ("body", Null)],
        HttpRequest => vec![
            ("method", String),
            ("url", String),
            ("body", Null),
            ("headers", Map),
        ],
        UrlParse => vec![("input", String)],
        UrlJoin => vec![("base", String), ("path", String)],

        // TCP
        TcpConnect => vec![("host", String), ("port", Null)],
        TcpWrite => vec![("conn_id", String), ("data", Null)],
        TcpRead => vec![("conn_id", String)],
        TcpClose => vec![("conn_id", String)],
        TcpBind => vec![("address", String), ("port", Null)],
        TcpAccept => vec![("listener_id", String)],
        TcpServerClose => vec![("listener_id", String)],

        // UDP
        UdpBind => vec![("address", String), ("port", Null)],
        UdpSendTo => vec![
            ("socket_id", String),
            ("data", Null),
            ("address", String),
            ("port", Null),
        ],
        UdpRecvFrom => vec![("socket_id", String)],
        UdpClose => vec![("socket_id", String)],

        // WebSocket
        WsConnect => vec![("url", String)],
        WsSend => vec![("conn_id", String), ("message", Null)],
        WsReceive => vec![("conn_id", String)],
        WsClose => vec![("conn_id", String)],

        // SSE
        SseConnect => vec![("url", String)],
        SseReadEvent => vec![("conn_id", String)],
        SseClose => vec![("conn_id", String)],

        // HTTP Server
        HttpServerStart => vec![("address", String), ("port", Null)],
        HttpServerReceive => vec![("server_id", String)],
        HttpServerRespond => vec![("client_id", String), ("status", Null), ("body", Null)],
        HttpServerStop => vec![("server_id", String)],

        // Certificate
        CertGenerate | CertSelfSigned => vec![("cn", String)],
        CertParse | CertInfo => vec![("pem", String)],
        CertVerify => vec![("pem", String)],
        KeyGenerate => vec![],

        // Path
        PathJoin => vec![("a", String), ("b", String)],
        PathBasename | PathDirname | PathExtension | PathStem | PathIsAbsolute | PathNormalize
        | PathSplit | PathParent => vec![("input", String)],
        PathWithExtension => vec![("input", String), ("extension", String)],

        // YAML/TOML
        YamlParse | YamlStringify | YamlValidate | YamlToJson | YamlFromJson | TomlParse
        | TomlStringify => vec![("input", Null)],
        YamlMerge => vec![("a", String), ("b", String)],

        // CSV
        CsvParse | CsvStringify | CsvHeaders | CsvParseRows => vec![("input", Null)],

        // Regex Extended
        RegexSplit | RegexTest | RegexCaptures | RegexFindAll => {
            vec![("input", String), ("pattern", String)]
        }
        RegexEscape => vec![("input", String)],

        // UUID
        UuidV4 | UuidNil => vec![],
        UuidParse | UuidIsValid => vec![("input", String)],

        // Crypto Extended
        HashSha512 | HashCrc32 => vec![("input", String)],
        HmacSha256 => vec![("input", String), ("key", String)],
        ConstantTimeEq => vec![("a", Null), ("b", Null)],

        // Compress
        CompressZstd | DecompressZstd | CompressLz4 | DecompressLz4 => vec![("input", Null)],

        // Format
        FmtNumber | FmtBytes | FmtDuration | FmtHex | FmtBinary | FmtPercent => {
            vec![("value", Null)]
        }

        // Convert Extended
        ParseInt | ParseFloat => vec![("input", String)],

        // Time Extended
        Duration => vec![],
        Elapsed => vec![("timestamp", Null)],
        TimeSleep => vec![("duration", Null)],
        AddDuration | SubDuration => vec![("timestamp", Null), ("duration", Null)],
        TimeDiff => vec![("a", Null), ("b", Null)],
        StartOf | EndOf => vec![("input", Null)],

        // Stats
        StatsMean | StatsMedian | StatsMode | StatsVariance | StatsStdDev | StatsSum => {
            vec![("array", Array)]
        }
        StatsMinBy | StatsMaxBy => vec![("array", Array), ("key", String)],
        StatsPercentile => vec![("array", Array), ("percentile", Null)],
        StatsQuantile => vec![("array", Array), ("quantile", Null)],
        StatsCovariance | StatsCorrelation => vec![("a", Array), ("b", Array)],

        // Text
        TextWrap | TextDedent | TextIndent | TextPadLeft | TextPadRight | TextTruncate
        | TextSlug | TextCamelCase | TextSnakeCase | TextTitleCase => vec![("input", String)],

        // Encode
        HtmlEscape | HtmlUnescape | Base32Encode | Base32Decode => vec![("input", Null)],

        // Reflect
        ReflectTypeOf | ReflectTypeName | ReflectFields | ReflectCallable | ReflectArity
        | ReflectInspect => vec![("input", Null)],
        ReflectIsType => vec![("input", Null), ("type_name", String)],
        ReflectHasField => vec![("input", Null), ("field", String)],

        // Collections
        SetFrom | Counter | OrderedMap => vec![("array", Array)],
        SetUnion | SetIntersection | SetDifference | SetSymmetricDifference => {
            vec![("a", Array), ("b", Array)]
        }
        MostCommon => vec![("array", Array)],

        // Sort
        SortAsc | SortDesc | StableSort | IsSorted | SortReverse | SortBy | SortByKey => {
            vec![("array", Array)]
        }
        BinarySearch => vec![("array", Array), ("value", Null)],
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
        Sqrt | Negate | Abs | Round | Floor | Ceil | Not | BitNot => &["value"],

        // Unary: input (string + type conversion)
        Substring | Length | ToUpper | ToLower | Trim | TrimStart | TrimEnd | CharAt | PadStart
        | PadEnd | StringReverse | StringRepeat | StringLines | StringWords
        | ToString | ToInt64 | ToFloat64 | ToBool | ToBytes | FromBytes
        | ParseJson | ToJson => &["input"],

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

        // Control flow
        IfElse => &["condition", "then", "else"],
        Switch => &["value", "default"],
        Coalesce => &["a", "b"],
        TryCatch => &["input", "fallback"],
        Error => &["message"],

        // Array
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

        // Map
        MapGet => &["map", "key"],
        MapSet => &["map", "key", "value"],
        MapDelete => &["map", "key"],
        MapHas => &["map", "key"],
        MapKeys | MapValues | MapEntries | MapSize => &["map"],
        MapMerge => &["a", "b"],
        MapFromEntries => &["array"],
        MapUpdate => &["map", "key", "value"],

        // Bytes
        BytesLength => &["input"],
        BytesSlice => &["input"],
        BytesConcat => &["a", "b"],
        BytesContains => &["input", "search"],
        Base64Encode | Base64Decode => &["input"],

        // Iteration
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
        HashSha256 | HashBlake3 | HashMd5 | UrlEncode | UrlDecode | HexDecode => &["input"],
        HexEncode => &["input"],

        // String extended
        StringFormat => &["template", "values"],

        // Control Flow extended
        Assert => &["condition", "message"],
        DebugLog => &["input"],

        // Type Conversion extended
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

        // String
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

        // Random
        RandomInt | RandomFloat | RandomBool | RandomBytes | RandomUuid | RandomString => &[],
        RandomRange => &["a", "b"],
        RandomChoice | RandomShuffle | RandomSample => &["array"],

        // Filesystem
        FsRead | FsExists | FsList | FsMkdir | FsSize | FsIsFile | FsIsDir | FsRemove => {
            &["path"]
        }
        FsWrite | FsAppend => &["path", "content"],
        FsCopy | FsMove => &["source", "destination"],

        // Environment
        EnvGet | EnvHas => &["key"],
        EnvKeys | OsName | OsArch | ProcessPid | CurrentDir => &[],

        // Network
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

        // Certificate
        CertGenerate | CertSelfSigned => &["cn"],
        CertParse | CertInfo | CertVerify => &["pem"],
        KeyGenerate => &[],

        // Path
        PathJoin => &["a", "b"],
        PathBasename | PathDirname | PathExtension | PathStem | PathIsAbsolute | PathNormalize
        | PathSplit | PathParent => &["input"],
        PathWithExtension => &["input", "extension"],

        // YAML/TOML
        YamlParse | YamlStringify | YamlValidate | YamlToJson | YamlFromJson | TomlParse
        | TomlStringify => &["input"],
        YamlMerge => &["a", "b"],

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

        // Compress
        CompressZstd | DecompressZstd | CompressLz4 | DecompressLz4 => &["input"],

        // Format
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

        // Stats
        StatsMean | StatsMedian | StatsMode | StatsVariance | StatsStdDev | StatsSum => {
            &["array"]
        }
        StatsMinBy | StatsMaxBy => &["array", "key"],
        StatsPercentile => &["array", "percentile"],
        StatsQuantile => &["array", "quantile"],
        StatsCovariance | StatsCorrelation => &["a", "b"],

        // Text
        TextWrap | TextDedent | TextIndent | TextPadLeft | TextPadRight | TextTruncate
        | TextSlug | TextCamelCase | TextSnakeCase | TextTitleCase => &["input"],

        // Encode Extended
        HtmlEscape | HtmlUnescape | Base32Encode | Base32Decode => &["input"],

        // Reflect
        ReflectTypeOf | ReflectTypeName | ReflectFields | ReflectCallable | ReflectArity
        | ReflectInspect => &["input"],
        ReflectIsType => &["input", "type_name"],
        ReflectHasField => &["input", "field"],

        // Collections
        SetFrom | Counter | OrderedMap => &["array"],
        SetUnion | SetIntersection | SetDifference | SetSymmetricDifference => &["a", "b"],
        MostCommon => &["array"],

        // Sort
        SortAsc | SortDesc | StableSort | IsSorted | SortReverse | SortBy | SortByKey => {
            &["array"]
        }
        BinarySearch => &["array", "value"],

        // V2 Language Constructs
        FunctionDef | FunctionCall => &[],
        AsyncSpawn | AsyncAwait => &["input"],
        LoopGroup => &[],
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
