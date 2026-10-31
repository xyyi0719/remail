//! HTML 消毒（白名单式）。
//!
//! ## 目标
//!
//! 把外部来的 HTML（富文本编辑器粘贴、邮件正文、网页片段）变成一段
//! **只含白名单标签与白名单属性**的 HTML，危险内容（脚本、事件处理器、
//! 远程图片、`javascript:` 链接）连同其内容一起消失。
//!
//! ## 为什么要自己写
//!
//! 本 crate **不许加新依赖**，没有现成的消毒库可用（`ammonia` 之类都要
//! 额外引入 crate），所以这里自带一个小的、够用的解析器。
//!
//! ## 实现方式
//!
//! 一个**手写的标签扫描器**，不是完整的 HTML5 解析器：
//!
//! * 按字节扫描，遇到 `<` 才开始尝试解析标签；`<` 后面不是 ASCII 字母
//!   （`< 5`、`a < b`）就当普通文本处理。
//! * 属性按 HTML5 的规则切分：属性名读到空白 / `=` / `>` / `/` 为止；
//!   值分三种——双引号、单引号、无引号（无引号读到空白或 `>` 为止，
//!   和浏览器一致，所以 `href=x/onerror=alert(1)` 里的 `/` 属于**值**的一部分）。
//! * 标签名大小写不敏感（`<ScRiPt>` = `<script>`），属性名同样不敏感。
//! * 用一个**开元素栈**维护嵌套关系：结束标签匹配不到栈顶时，会把中间的
//!   元素隐式闭合后再闭合自己，输出因此总是良构的；栈里没有的结束标签
//!   （stray）直接丢掉。
//! * 实体引用（`&lt;` `&amp;` `&#106;` …）**不当成标签**：只有真正的 `<`
//!   才开始解析标签，因此 `&lt;script&gt;` 只会作为文本原样保留。
//!
//! ## 为什么禁掉 `img`
//!
//! 邮件正文里的远程图片是**跟踪像素**：`<img src="https://evil.example/p.gif">`
//! 会在邮件被打开时把「谁在什么时候读了这封邮件」送出去给发件方。
//! 邮件客户端默认不该加载远程图片，所以本消毒器把 `img` 整个去掉
//! （连 `alt` 文本一起没有）。
//!
//! ## 为什么属性只留这几个
//!
//! 只保留 `a[href]` `a[title]` `td/th[colspan rowspan]` `ol[start]`。
//! 其余一律删除，包括：
//!
//! * `style`：能用 CSS 做定位覆盖、点击劫持、外观欺骗；
//! * `class` / `id`：钓鱼邮件常用它们伪装成真实页面（配合外部 CSS 很容易骗人）；
//! * 所有 `on*` 事件处理器：`onerror` / `onload` / `onclick` 是脚本注入的入口；
//! * 其它一切属性（包括 `target`、`srcset`、`background` 之类）。
//!
//! ## 已知边界（畸形 HTML 可能绕过或被过度处理）
//!
//! * **不是完整的 HTML5 解析器**：不实现 HTML5 的错误恢复算法，
//!   `<table>` 里塞 `<div>`、表格「foster parenting」这类浏览器特有的
//!   修正规则一概不模拟。浏览器对同一段 HTML 的解析结果可能与我们不同。
//! * **输出是重新序列化过的**，标签闭合顺序被规整（隐式闭合），所以输出
//!   和输入的 DOM 结构不一定逐字对应——这是刻意的：输出越接近良构，
//!   浏览器二次解析时越不容易出现 mXSS。
//! * **未知标签连内容一起删**（白名单式的严格含义）：`<font>`、`<center>`、
//!   Word 导出的 `<o:p>` 等标签会让**它们包裹的内容也消失**；
//!   未闭合的未知标签（例如 `<o:p>` 没有对应的 `</o:p>`）会让**它之后的
//!   全部内容被丢弃**。这是刻意的安全取舍：宁可少显示，也不要留下
//!   「解析歧义」给浏览器。
//! * **实体只在属性值里解码**：文本里的实体原样保留（`&lt;` 仍是 `&lt;`），
//!   属性值里的实体先解码再判断协议、输出时重新转义。命名实体只认一小部分
//!   （`amp` `lt` `gt` `quot` `apos` `colon` `tab` `newline` …）和全部数字实体，
//!   不认识的命名实体会被转义成 `&amp;xxx;`（显示成字面量，属于观感差异）。
//! * **不处理 CSS**（`style` 属性本来就删），也不处理 `srcset` 之类的多 URL 属性
//!   （属性本来就删）。

/// 白名单：允许保留的标签。
const ALLOWED_TAGS: &[&str] = &[
    "p",
    "br",
    "b",
    "strong",
    "i",
    "em",
    "u",
    "s",
    "ul",
    "ol",
    "li",
    "a",
    "blockquote",
    "code",
    "pre",
    "span",
    "div",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "hr",
    "table",
    "thead",
    "tbody",
    "tr",
    "td",
    "th",
];

/// HTML5 的空元素：没有结束标签、也没有内容。
///
/// 这些标签在消毒时**只删标签本身**（不扫描「对应结束标签」），
/// 否则 `<img>` 之类找不到结束标签会把后面全部内容吞掉。
const VOID_TAGS: &[&str] = &[
    "area", "base", "br", "col", "embed", "frame", "hr", "img", "input", "link", "meta", "param",
    "source", "track", "wbr",
];

/// 必须**连内容一起删掉**的标签（脚本、样式、框架、表单、SVG/MathML…）。
///
/// 这些标签里的内容本身就有副作用或本身就是攻击载荷，
/// 只删标签留内容等于把 `<script>alert(1)</script>` 变成 `alert(1)`。
///
/// 公开出来是为了让调用方能解释「为什么这封邮件里这段内容不见了」。
pub const DROP_WITH_CONTENT: &[&str] = &[
    "script", "style", "iframe", "object", "embed", "form", "input", "button", "textarea",
    "select", "svg", "math", "template", "noscript", "frame", "frameset", "applet", "base", "link",
    "meta",
];

/// `colspan` / `rowspan` 的取值上限。
///
/// 规范上这两个值没有上限，但 `colspan="2000000000"` 会让排版引擎去做
/// 荒谬的计算，属于资源耗尽的一种。所以这里夹一个上限，超了就丢掉这个属性。
const MAX_SPAN: u32 = 1000;

/// `ol[start]` 的取值范围上限（绝对值）。
const MAX_OL_START: i64 = 100_000;

/// 允许保留的 URL 协议。
const ALLOWED_SCHEMES: &[&str] = &["http", "https", "mailto"];

/// 消毒一段 HTML。
///
/// 返回**重新序列化**过的安全 HTML：只含白名单标签与白名单属性，
/// 文本与属性值都做了转义。
///
/// ```
/// use xremail_core::html::sanitize_html;
///
/// let clean = sanitize_html(r#"<p onclick="x()">hi <script>alert(1)</script></p>"#);
/// assert!(clean.contains("<p>hi"));
/// assert!(!clean.contains("onclick"));
/// assert!(!clean.contains("alert"));
/// ```
pub fn sanitize_html(input: &str) -> String {
    Sanitizer::default().run(input)
}

/// 把 HTML 变成纯文本。
///
/// ## 为什么要它
///
/// 真实邮箱里的邮件**大多是 HTML-only**（实测 163：纯文本部分为**空**，
/// 只有 `text/html`）。如果可搜索的正文只认 `text_plain`，
/// 这些邮件的正文**一个字都搜不到**——搜索等于只搜标题。
///
/// ## 做法
///
/// 先过一遍 [`sanitize_html`]（顺带去掉脚本、样式、远程图片），
/// 再把标签换成「块级换行 / 单元格制表符」，最后解实体、压空白。
///
/// ## 已知取舍
///
/// * `<pre>` 里的缩进会被空白压缩抹平——这是给搜索用的**投影**，
///   不是渲染用的文本。
/// * 表格单元格之间是制表符；行与行之间是换行。
pub fn html_to_text(input: &str) -> String {
    let clean = sanitize_html(input);
    let mut out = String::with_capacity(clean.len() / 2);
    let mut rest = clean.as_str();

    while !rest.is_empty() {
        match rest.find('<') {
            Some(pos) => {
                out.push_str(&rest[..pos]);
                let after = &rest[pos + 1..];
                match after.find('>') {
                    Some(end) => {
                        out.push_str(block_separator(&after[..end]));
                        rest = &after[end + 1..];
                    }
                    None => {
                        // 没有 `>`：这个 `<` 是普通文本（消毒器的输出里不会出现，
                        // 但这里不假设它一定不会出现）。
                        out.push('<');
                        rest = after;
                    }
                }
            }
            None => {
                out.push_str(rest);
                rest = "";
            }
        }
    }

    normalize_text(&decode_entities(&out))
}

/// 标签该换成什么分隔符。
///
/// 传入的是**去掉尖括号**的标签内容（`p` / `/p` / `br /` …）。
///
/// 规则：
///
/// * 块级元素的**结束**标签 → 换行（段落、列表项、表格行各占一行）；
/// * `<br>` → 换行；`<hr>` → 换行；
/// * 单元格**开始**标签 → 制表符（结束标签不加分隔符，否则会出现双制表符）；
/// * 其余一律不加分隔符。
fn block_separator(tag_text: &str) -> &'static str {
    let trimmed = tag_text.trim();
    let is_end = trimmed.starts_with('/');
    let name = trimmed
        .trim_start_matches('/')
        .split(|ch: char| ch.is_whitespace() || ch == '/')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();

    if is_end {
        return match name.as_str() {
            "p" | "div" | "li" | "ul" | "ol" | "tr" | "table" | "tbody" | "thead"
            | "blockquote" | "pre" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => "\n",
            _ => "",
        };
    }

    match name.as_str() {
        "br" | "hr" => "\n",
        "td" | "th" => "\t",
        _ => "",
    }
}

/// 压空白：行尾空格、连续空行、不间断空格。
fn normalize_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    // 上一行是空行：这一行前面要**多空一行**（段落分隔）。
    let mut pending_blank = false;

    for line in text.split('\n') {
        // 不间断空格当普通空格。
        let line = line.replace('\u{a0}', " ");
        // 行内连续空白压成一个空格；行尾空格去掉。
        // **制表符保留**：它是单元格之间的分隔符。
        let mut collapsed = String::with_capacity(line.len());
        let mut pending_space = false;
        for ch in line.chars() {
            if ch.is_whitespace() && ch != '\t' {
                pending_space = true;
                continue;
            }
            if pending_space && !collapsed.is_empty() {
                collapsed.push(' ');
            }
            pending_space = false;
            collapsed.push(ch);
        }
        // 行首的制表符（第一列前面那个）去掉。
        let collapsed = collapsed.trim_start_matches('\t');

        if collapsed.is_empty() {
            pending_blank = true;
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
            if pending_blank {
                out.push('\n');
            }
        }
        pending_blank = false;
        out.push_str(collapsed);
    }

    out
}

/// 属性值的类型：决定怎么校验、要不要保留。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttrKind {
    /// URL：只允许 `http` / `https` / `mailto`。
    Url,
    /// 普通文本：直接保留（转义后输出）。
    Text,
    /// 正整数（`colspan` / `rowspan`），且不超过 [`MAX_SPAN`]。
    PositiveInt,
    /// 整数（`ol[start]`），绝对值不超过 [`MAX_OL_START`]。
    SignedInt,
}

/// 某个标签允许保留哪些属性。
fn allowed_attributes(tag: &str) -> &'static [(&'static str, AttrKind)] {
    match tag {
        "a" => &[("href", AttrKind::Url), ("title", AttrKind::Text)],
        "td" | "th" => &[
            ("colspan", AttrKind::PositiveInt),
            ("rowspan", AttrKind::PositiveInt),
        ],
        "ol" => &[("start", AttrKind::SignedInt)],
        _ => &[],
    }
}

/// 在白名单里找标签名（返回静态字符串，方便放进开元素栈）。
fn allowed_tag(name: &str) -> Option<&'static str> {
    ALLOWED_TAGS.iter().copied().find(|tag| *tag == name)
}

/// 消毒器。
#[derive(Debug, Default)]
struct Sanitizer {
    /// 输出缓冲。
    out: String,
    /// 当前打开的（白名单）元素栈，用于正确处理嵌套与错乱闭合。
    stack: Vec<&'static str>,
}

impl Sanitizer {
    /// 主循环。
    fn run(mut self, input: &str) -> String {
        let bytes = input.as_bytes();
        let mut i = 0usize;

        while i < bytes.len() {
            if bytes[i] == b'<' {
                match self.handle_markup(bytes, i) {
                    Some(next) => i = next,
                    // `<` 后面不是标签开头 → 当普通文本。
                    None => {
                        self.out.push_str("&lt;");
                        i += 1;
                    }
                }
            } else {
                let start = i;
                while i < bytes.len() && bytes[i] != b'<' {
                    i += 1;
                }
                push_escaped_text(&mut self.out, &input[start..i]);
            }
        }

        // 收尾：把还没闭合的元素补上结束标签，保证输出良构。
        while let Some(tag) = self.stack.pop() {
            self.out.push_str("</");
            self.out.push_str(tag);
            self.out.push('>');
        }

        self.out
    }

    /// 处理 `<` 开头的标记，返回下一个位置；`None` 表示「这不是标签」。
    fn handle_markup(&mut self, bytes: &[u8], pos: usize) -> Option<usize> {
        let rest = &bytes[pos..];
        if rest.len() < 2 {
            return None;
        }

        match rest[1] {
            // 注释 / DOCTYPE / CDATA / 伪注释：全部丢弃。
            b'!' => Some(pos + skip_bang(rest)),
            // `<?php ... ?>` 之类的伪注释：丢弃到 `>`。
            b'?' => Some(pos + skip_to_gt(rest)),
            b'/' => {
                let (name, after) = parse_tag_name(rest, 2)?;
                if ALLOWED_TAGS.contains(&name.as_str()) {
                    self.close_tag(&name);
                }
                Some(pos + after + skip_to_gt(&rest[after..]))
            }
            c if c.is_ascii_alphabetic() => {
                let (name, attrs, self_closing, after) = parse_start_tag(rest)?;
                let content_start = pos + after;
                match allowed_tag(&name) {
                    Some(tag) => {
                        self.emit_start_tag(tag, &attrs, self_closing);
                        Some(content_start)
                    }
                    None => {
                        // 不在白名单，分两种：
                        //
                        // * **危险标签**（脚本、样式、框架、表单…）→ 连内容一起删。
                        //   这些标签里的内容本身就有副作用，或者本身就是载荷，
                        //   只删标签留内容等于把 `<script>alert(1)</script>`
                        //   变成一段普通文本。
                        // * **其余未知标签**（`html` / `body` / `font` / `center` …）
                        //   → **只删标签，内容留着**。
                        //
                        // 后半条是必须的：真实邮件的正文**全都被** `html` / `body`
                        // 包着（163 的邮件就是 `<html><body>…`），
                        // 要是连内容一起删，整封邮件**一个字都不剩**。
                        if DROP_WITH_CONTENT.contains(&name.as_str()) {
                            Some(skip_element(&name, bytes, content_start))
                        } else {
                            Some(content_start)
                        }
                    }
                }
            }
            _ => None,
        }
    }

    /// 输出一个允许保留的开始标签。
    fn emit_start_tag(
        &mut self,
        tag: &'static str,
        attrs: &[(String, String)],
        self_closing: bool,
    ) {
        // 隐式闭合：HTML5 里有些标签的开始标签会自动关掉前一个同类的。
        self.implicit_close(tag);

        self.out.push('<');
        self.out.push_str(tag);
        self.emit_attributes(tag, attrs);
        self.out.push('>');

        if VOID_TAGS.contains(&tag) {
            // 空元素没有结束标签。
        } else if self_closing {
            // `<div/>` → 输出成 `<div></div>`（HTML5 不认自闭合的 div）。
            self.out.push_str("</");
            self.out.push_str(tag);
            self.out.push('>');
        } else {
            self.stack.push(tag);
        }
    }

    /// 输出允许保留的属性。
    fn emit_attributes(&mut self, tag: &str, attrs: &[(String, String)]) {
        for (attr_name, kind) in allowed_attributes(tag) {
            // 同名属性取**第一个**（HTML5 的规则：后来的重复属性被丢弃）。
            let Some((_, raw)) = attrs.iter().find(|(n, _)| n.as_str() == *attr_name) else {
                continue;
            };

            // 属性值里的实体先解码：浏览器解析 URL 时看到的是解码后的值，
            // 所以判断协议也必须在解码之后做（`jav&#x09;ascript:` 是个经典绕过）。
            let decoded = decode_entities(raw);

            let value = match kind {
                AttrKind::Url => {
                    if !is_allowed_url(&decoded) {
                        continue;
                    }
                    decoded.trim().to_string()
                }
                AttrKind::Text => decoded,
                AttrKind::PositiveInt => match decoded.trim().parse::<u32>() {
                    Ok(v) if (1..=MAX_SPAN).contains(&v) => v.to_string(),
                    _ => continue,
                },
                AttrKind::SignedInt => match decoded.trim().parse::<i64>() {
                    Ok(v) if v.abs() <= MAX_OL_START => v.to_string(),
                    _ => continue,
                },
            };

            self.out.push(' ');
            self.out.push_str(attr_name);
            self.out.push_str("=\"");
            push_escaped_attr_value(&mut self.out, &value);
            self.out.push('"');
        }
    }

    /// 结束标签：匹配栈里的元素，必要时先隐式闭合中间的元素。
    fn close_tag(&mut self, name: &str) {
        let Some(index) = self.stack.iter().rposition(|t| *t == name) else {
            // 没有对应的开始标签 → 丢弃（stray end tag）。
            return;
        };

        while self.stack.len() > index + 1 {
            let tag = self.stack.pop().expect("栈非空");
            self.out.push_str("</");
            self.out.push_str(tag);
            self.out.push('>');
        }

        let tag = self.stack.pop().expect("栈非空");
        self.out.push_str("</");
        self.out.push_str(tag);
        self.out.push('>');
    }

    /// 开始标签触发的隐式闭合。
    fn implicit_close(&mut self, tag: &str) {
        loop {
            let Some(top) = self.stack.last().copied() else {
                return;
            };

            let implied = implied_end_tags(tag).contains(&top);
            // `<p>` 会被任何块级元素的开始标签关掉。
            let closes_p = top == "p" && closes_paragraph(tag);

            if implied || closes_p {
                let tag = self.stack.pop().expect("栈非空");
                self.out.push_str("</");
                self.out.push_str(tag);
                self.out.push('>');
            } else {
                return;
            }
        }
    }
}

/// 某个开始标签会隐式关掉哪些标签（HTML5 的可选结束标签规则）。
fn implied_end_tags(tag: &str) -> &'static [&'static str] {
    match tag {
        "li" => &["li"],
        "p" => &["p"],
        "td" | "th" => &["td", "th"],
        "tr" => &["tr", "td", "th"],
        "thead" | "tbody" | "tfoot" => &["thead", "tbody", "tfoot", "tr", "td", "th"],
        _ => &[],
    }
}

/// 哪些元素的开始标签会关掉一个开着的 `<p>`。
fn closes_paragraph(tag: &str) -> bool {
    matches!(
        tag,
        "p" | "div"
            | "ul"
            | "ol"
            | "li"
            | "table"
            | "thead"
            | "tbody"
            | "tr"
            | "td"
            | "th"
            | "blockquote"
            | "pre"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "hr"
    )
}

/// 解析标签名，返回 `(小写名字, 名字结束后的偏移)`。
fn parse_tag_name(bytes: &[u8], start: usize) -> Option<(String, usize)> {
    if start >= bytes.len() || !bytes[start].is_ascii_alphabetic() {
        return None;
    }
    let mut i = start;
    while i < bytes.len()
        && (bytes[i].is_ascii_alphanumeric()
            || bytes[i] == b'-'
            || bytes[i] == b'_'
            || bytes[i] == b':')
    {
        i += 1;
    }
    let name: Vec<u8> = bytes[start..i]
        .iter()
        .map(|b| b.to_ascii_lowercase())
        .collect();
    Some((String::from_utf8_lossy(&name).into_owned(), i))
}

/// 属性列表：`[(属性名, 原始属性值)]`，属性名已转小写。
type Attributes = Vec<(String, String)>;

/// 开始标签的解析结果：`(名字, 属性列表, 是否自闭合, 标签结束后的偏移)`。
type ParsedStartTag = (String, Attributes, bool, usize);

/// 解析一个开始标签，返回 `(名字, 属性列表, 是否自闭合, 标签结束后的偏移)`。
fn parse_start_tag(bytes: &[u8]) -> Option<ParsedStartTag> {
    let (name, mut i) = parse_tag_name(bytes, 1)?;
    let mut attrs: Vec<(String, String)> = Vec::new();
    let mut self_closing = false;

    loop {
        while i < bytes.len() && is_ws(bytes[i]) {
            i += 1;
        }

        if i >= bytes.len() {
            // 标签没有闭合：整段丢弃（当作不是标签）。
            return None;
        }

        if bytes[i] == b'>' {
            i += 1;
            break;
        }

        if bytes[i] == b'/' {
            // `/>` 是自闭合；孤立的 `/` 直接跳过。
            if i + 1 < bytes.len() && bytes[i + 1] == b'>' {
                self_closing = true;
                i += 2;
                break;
            }
            i += 1;
            continue;
        }

        // 属性名：读到空白 / `=` / `>` / `/` 为止。
        let name_start = i;
        while i < bytes.len()
            && !is_ws(bytes[i])
            && bytes[i] != b'='
            && bytes[i] != b'>'
            && bytes[i] != b'/'
        {
            i += 1;
        }
        if i == name_start {
            // 不可能是真字符，跳过以免死循环。
            i += 1;
            continue;
        }
        let attr_name = bytes[name_start..i]
            .iter()
            .map(|b| b.to_ascii_lowercase())
            .collect::<Vec<u8>>();
        let attr_name = String::from_utf8_lossy(&attr_name).into_owned();

        while i < bytes.len() && is_ws(bytes[i]) {
            i += 1;
        }

        let value = if i < bytes.len() && bytes[i] == b'=' {
            i += 1;
            while i < bytes.len() && is_ws(bytes[i]) {
                i += 1;
            }
            if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
                let quote = bytes[i];
                i += 1;
                let value_start = i;
                while i < bytes.len() && bytes[i] != quote {
                    i += 1;
                }
                let value = bytes[value_start..i].to_vec();
                if i < bytes.len() {
                    i += 1;
                }
                String::from_utf8_lossy(&value).into_owned()
            } else {
                // 无引号的值：读到空白或 `>` 为止（`/` 属于值的一部分，
                // 和浏览器一致，所以 `href=x/onerror=alert(1)` 是一个属性）。
                let value_start = i;
                while i < bytes.len() && !is_ws(bytes[i]) && bytes[i] != b'>' {
                    i += 1;
                }
                String::from_utf8_lossy(&bytes[value_start..i]).into_owned()
            }
        } else {
            String::new()
        };

        // 同名属性只保留第一个。
        if !attrs.iter().any(|(n, _)| n == &attr_name) {
            attrs.push((attr_name, value));
        }
    }

    Some((name, attrs, self_closing, i))
}

/// 跳过 `<!-- ... -->` / `<!DOCTYPE ...>` / `<![CDATA[...]]>` 之类，返回跳过的长度。
fn skip_bang(rest: &[u8]) -> usize {
    if rest.starts_with(b"<!--") {
        // 注释：浏览器在 `-->` 或 `--!>` 处结束，我们只认 `-->`。
        // 少认一种只会多删内容（安全方向），不会少删。
        return match find_bytes(rest, b"-->") {
            Some(index) => index + 3,
            None => rest.len(),
        };
    }
    if rest.starts_with(b"<![CDATA[") {
        return match find_bytes(rest, b"]]>") {
            Some(index) => index + 3,
            None => rest.len(),
        };
    }
    // DOCTYPE / 伪注释：丢弃到 `>`。
    skip_to_gt(rest)
}

/// 丢弃到 `>` 为止，返回跳过的长度（**含** `>`）。
fn skip_to_gt(bytes: &[u8]) -> usize {
    match bytes.iter().position(|b| *b == b'>') {
        Some(index) => index + 1,
        None => bytes.len(),
    }
}

/// 找到字节串 `needle` 第一次出现的位置。
fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// 跳过整个元素（连内容一起删），返回结束后的偏移。
///
/// * `script` / `style` 是**原始文本**元素：内容里出现 `<` 不算标签，
///   只有 `</script` / `</style` 才结束它们——这正是浏览器的行为。
/// * 其余元素按普通标签计数：同名开始标签会加深一层。
/// * 找不到结束标签时一直删到文档末尾（浏览器的 raw-text 元素也是如此）。
fn skip_element(name: &str, bytes: &[u8], start: usize) -> usize {
    let raw_text = name == "script" || name == "style";
    let mut depth = 1usize;
    let mut i = start;

    while i < bytes.len() {
        let Some(offset) = bytes[i..].iter().position(|b| *b == b'<') else {
            return bytes.len();
        };
        let lt = i + offset;
        let rest = &bytes[lt..];

        if rest.starts_with(b"<!--") {
            i = lt + skip_bang(rest);
            continue;
        }

        if rest.len() > 1 && rest[1] == b'/' {
            if let Some((tag_name, after)) = parse_tag_name(rest, 2)
                && tag_name == name
            {
                depth -= 1;
                if depth == 0 {
                    return lt + after + skip_to_gt(&rest[after..]);
                }
            }
            i = lt + 1;
            continue;
        }

        if let Some((tag_name, _)) = parse_tag_name(rest, 1)
            && tag_name == name
            && !raw_text
        {
            let self_closing = parse_start_tag(rest)
                .map(|(_, _, closing, _)| closing)
                .unwrap_or(false);
            if !self_closing {
                depth += 1;
            }
        }

        i = lt + 1;
    }

    bytes.len()
}

/// 判断一个 URL 是否用了允许的协议。
///
/// 规则：先去掉所有控制字符（浏览器解析 URL 时会去掉 TAB/LF/CR），
/// 再去掉首尾空白，然后看 `:` 之前的部分是不是 `http` / `https` / `mailto`。
///
/// 没有协议的相对地址也**不允许**：邮件正文里没有「基地址」，
/// 相对链接点了必然是坏的，不如直接去掉。
fn is_allowed_url(value: &str) -> bool {
    let cleaned: String = value.chars().filter(|c| !c.is_control()).collect();
    let cleaned = cleaned.trim();

    let Some(colon) = cleaned.find(':') else {
        return false;
    };
    let scheme = &cleaned[..colon];
    if scheme.is_empty() {
        return false;
    }
    let mut chars = scheme.chars();
    let first = chars.next().expect("非空");
    if !first.is_ascii_alphabetic() {
        return false;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.') {
        return false;
    }

    ALLOWED_SCHEMES.contains(&scheme.to_ascii_lowercase().as_str())
}

/// 判断是否是 HTML 空白字符。
fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | b'\x0c')
}

/// 转义文本：`<` `>` 变成实体，`&` 只在**不是**合法实体引用时转义。
///
/// 合法的实体引用原样保留（`&lt;` 仍然是 `&lt;`），
/// 这样 `&lt;script&gt;` 在正文里显示成字面的 `<script>`，而不是被当成标签。
fn push_escaped_text(out: &mut String, text: &str) {
    let bytes = text.as_bytes();
    let mut i = 0usize;
    let mut chunk_start = 0usize;

    while i < bytes.len() {
        match bytes[i] {
            b'<' => {
                out.push_str(&text[chunk_start..i]);
                out.push_str("&lt;");
                i += 1;
                chunk_start = i;
            }
            b'>' => {
                out.push_str(&text[chunk_start..i]);
                out.push_str("&gt;");
                i += 1;
                chunk_start = i;
            }
            b'&' => match entity_len(&bytes[i..]) {
                // 合法实体：原样保留。
                Some(len) => i += len,
                None => {
                    out.push_str(&text[chunk_start..i]);
                    out.push_str("&amp;");
                    i += 1;
                    chunk_start = i;
                }
            },
            _ => i += 1,
        }
    }

    out.push_str(&text[chunk_start..]);
}

/// 转义属性值：把 `&` `<` `>` `"` 和控制字符处理掉。
///
/// 调用方传入的应当是**已经解码**过的值（见 [`decode_entities`]），
/// 这里重新编码，保证输出里不会出现能逃出引号的内容。
fn push_escaped_attr_value(out: &mut String, value: &str) {
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\u{0}'..='\u{1f}' | '\u{7f}' => {}
            _ => out.push(ch),
        }
    }
}

/// 如果 `bytes` 开头是一个合法的实体引用，返回它的长度。
fn entity_len(bytes: &[u8]) -> Option<usize> {
    if bytes.first() != Some(&b'&') {
        return None;
    }
    let body = &bytes[1..];

    if body.first() == Some(&b'#') {
        let hex = body.len() > 1 && (body[1] == b'x' || body[1] == b'X');
        let digits_start = if hex { 2 } else { 1 };
        let mut i = digits_start;
        while i < body.len() && body[i].is_ascii_digit() {
            i += 1;
        }
        if hex {
            while i < body.len() && body[i].is_ascii_hexdigit() {
                i += 1;
            }
        }
        if i == digits_start || i >= body.len() || body[i] != b';' {
            return None;
        }
        return Some(1 + i + 1);
    }

    let mut i = 0usize;
    while i < body.len() && body[i].is_ascii_alphanumeric() {
        i += 1;
    }
    if i == 0 || i >= body.len() || body[i] != b';' {
        return None;
    }
    // 名字最长 32 个字符（HTML5 的限制）。
    if i > 32 {
        return None;
    }
    Some(1 + i + 1)
}

/// 解码实体引用（数字实体 + 一小部分命名实体）。
///
/// 命名实体只认下面这些——覆盖了所有能用来拼出 `javascript:` 的那几个
/// （`colon` `Tab` `NewLine` …），剩下的不认只是显示成字面量。
fn decode_entities(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0usize;

    while i < bytes.len() {
        if bytes[i] == b'&'
            && let Some((replacement, len)) = decode_entity(&bytes[i..])
        {
            out.push_str(&replacement);
            i += len;
            continue;
        }
        // 原样复制一个字符（可能是多字节的 UTF-8）。
        let len = utf8_len(bytes[i]);
        let end = (i + len).min(text.len());
        if let Some(chunk) = text.get(i..end) {
            out.push_str(chunk);
        }
        i = end.max(i + 1);
    }

    out
}

/// 解码单个实体引用，返回 `(替换文本, 长度)`。
fn decode_entity(bytes: &[u8]) -> Option<(String, usize)> {
    let body = &bytes[1..]; // 去掉 `&`

    if body.first() == Some(&b'#') {
        let hex = body.len() > 1 && (body[1] == b'x' || body[1] == b'X');
        let digits_start = if hex { 2 } else { 1 };
        let mut i = digits_start;
        while i < body.len()
            && (if hex {
                body[i].is_ascii_hexdigit()
            } else {
                body[i].is_ascii_digit()
            })
        {
            i += 1;
        }
        if i == digits_start || i >= body.len() || body[i] != b';' {
            return None;
        }
        let digits = std::str::from_utf8(&body[digits_start..i]).ok()?;
        let code = if hex {
            u32::from_str_radix(digits, 16).ok()?
        } else {
            digits.parse::<u32>().ok()?
        };
        let ch = char::from_u32(code)?;
        return Some((ch.to_string(), 1 + i + 1));
    }

    let mut i = 0usize;
    while i < body.len() && body[i].is_ascii_alphanumeric() {
        i += 1;
    }
    if i == 0 || i >= body.len() || body[i] != b';' {
        return None;
    }
    let name = std::str::from_utf8(&body[..i]).ok()?;
    let replacement = match name {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        "nbsp" => "\u{a0}",
        "colon" => ":",
        "tab" | "Tab" => "\t",
        "newline" | "NewLine" => "\n",
        "sol" => "/",
        "lpar" => "(",
        "rpar" => ")",
        "semi" => ";",
        "num" => "#",
        "excl" => "!",
        "quest" => "?",
        "equals" => "=",
        "plus" => "+",
        "percnt" => "%",
        "commat" => "@",
        "dollar" => "$",
        "midast" => "*",
        "period" => ".",
        "comma" => ",",
        "verbar" => "|",
        "lowbar" => "_",
        _ => return None,
    };
    Some((replacement.to_string(), 1 + i + 1))
}

/// UTF-8 字符的字节长度。
fn utf8_len(first: u8) -> usize {
    match first {
        0x00..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf7 => 4,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── 攻击样本 ────────────────────────────────────────────────────────

    #[test]
    fn removes_script_element_with_its_content() {
        let out = sanitize_html("<p>ok</p><script>alert(1)</script>");
        assert!(!out.contains("script"), "{out}");
        assert!(!out.contains("alert"), "{out}");
        assert_eq!(out, "<p>ok</p>");
    }

    #[test]
    fn removes_script_regardless_of_case() {
        let out = sanitize_html("<ScRiPt>alert(1)</ScRiPt><p>ok</p>");
        assert!(!out.to_lowercase().contains("script"), "{out}");
        assert!(!out.contains("alert"), "{out}");
        assert!(out.contains("<p>ok</p>"), "{out}");
    }

    #[test]
    fn removes_img_with_tracking_pixel_and_event_handler() {
        let out = sanitize_html(r#"<img src=x onerror=alert(1)>"#);
        assert!(!out.contains("img"), "{out}");
        assert!(!out.contains("onerror"), "{out}");
        assert!(!out.contains("alert"), "{out}");
        assert_eq!(out, "");
    }

    #[test]
    fn removes_iframe_with_content() {
        let out = sanitize_html(r#"<iframe src="https://evil.example/"></iframe>"#);
        assert!(!out.contains("iframe"), "{out}");
        assert!(!out.contains("evil.example"), "{out}");
    }

    #[test]
    fn removes_svg_with_content() {
        let out = sanitize_html(r#"<svg onload=alert(1)><circle/></svg><p>ok</p>"#);
        assert!(!out.contains("svg"), "{out}");
        assert!(!out.contains("onload"), "{out}");
        assert!(!out.contains("alert"), "{out}");
        assert!(out.contains("<p>ok</p>"), "{out}");
    }

    #[test]
    fn removes_math_mglyph_style_bypass_payload() {
        // 经典 mXSS 载荷：<math><mtext><table><mglyph><style><!--</style>
        //              <img title="--><img src=1 onerror=alert(1)>">
        let payload = r#"<math><mtext><table><mglyph><style><!--</style><img title="--><img src=1 onerror=alert(1)>">"#;
        let out = sanitize_html(payload);
        assert!(!out.contains("onerror"), "{out}");
        assert!(!out.contains("alert"), "{out}");
        assert!(!out.contains("<math"), "{out}");
        assert!(!out.contains("<style"), "{out}");
    }

    #[test]
    fn strips_javascript_url_but_keeps_link_text() {
        let out = sanitize_html(r#"<a href="javascript:alert(1)">click</a>"#);
        assert!(!out.contains("javascript"), "{out}");
        // 链接文字与链接本身保留（只是去掉了危险的 href）。
        assert!(out.contains("click"), "{out}");
        assert!(out.contains("<a>"), "{out}");
    }

    #[test]
    fn strips_vbscript_and_data_urls() {
        for payload in [
            r#"<a href="vbscript:msgbox(1)">x</a>"#,
            r#"<a href="data:text/html;base64,PHNjcmlwdD4=">x</a>"#,
            r#"<a href="data:text/html,<script>alert(1)</script>">x</a>"#,
        ] {
            let out = sanitize_html(payload);
            assert!(!out.contains("vbscript"), "{out}");
            assert!(!out.contains("data:"), "{out}");
            assert!(!out.contains("script"), "{out}");
            assert!(out.contains("x"), "{out}");
        }
    }

    #[test]
    fn strips_javascript_url_with_whitespace_and_entities() {
        // 大小写混写 + 前导空白 + 实体编码的 TAB。
        for payload in [
            r#"<a href="  JAVASCRIPT:alert(1)">x</a>"#,
            r#"<a href="jav&#x09;ascript:alert(1)">x</a>"#,
            r#"<a href="java&Tab;script:alert(1)">x</a>"#,
            r#"<a href="jav&NewLine;ascript:alert(1)">x</a>"#,
            r#"<a href="&#106;avascript:alert(1)">x</a>"#,
        ] {
            let out = sanitize_html(payload);
            assert!(!out.to_lowercase().contains("javascript:"), "{out}");
            assert!(!out.contains("alert"), "{out}");
        }
    }

    #[test]
    fn strips_event_handler_attributes() {
        let out = sanitize_html(
            r#"<div onclick="alert(1)" onmouseover=alert(2) class="x" id="y">t</div>"#,
        );
        assert!(!out.contains("onclick"), "{out}");
        assert!(!out.contains("onmouseover"), "{out}");
        assert!(!out.contains("alert"), "{out}");
        assert!(!out.contains("class"), "{out}");
        assert!(!out.contains("id="), "{out}");
        // 标签和文字保留。
        assert!(out.contains("<div>t</div>"), "{out}");
    }

    #[test]
    fn strips_style_attribute_but_keeps_element() {
        let out = sanitize_html(
            r#"<div style="position:fixed;top:0;left:0;width:100%;height:100%">x</div>"#,
        );
        assert!(!out.contains("style"), "{out}");
        assert!(!out.contains("position"), "{out}");
        assert_eq!(out, "<div>x</div>");
    }

    #[test]
    fn handles_unquoted_attributes() {
        let out = sanitize_html("<a href=javascript:alert(1) onclick=alert(2)>x</a>");
        assert!(!out.contains("javascript"), "{out}");
        assert!(!out.contains("onclick"), "{out}");
        assert!(out.contains("x"), "{out}");
    }

    #[test]
    fn removes_style_element_and_form_controls() {
        let out = sanitize_html(
            r#"<style>body{background:url(javascript:alert(1))}</style><form action="/x"><input name="a"><button>go</button></form><p>ok</p>"#,
        );
        assert!(!out.contains("style"), "{out}");
        assert!(!out.contains("javascript"), "{out}");
        assert!(!out.contains("<form"), "{out}");
        assert!(!out.contains("<input"), "{out}");
        assert!(!out.contains("<button"), "{out}");
        assert!(out.contains("<p>ok</p>"), "{out}");
    }

    #[test]
    fn removes_html_comments() {
        let out = sanitize_html("<!-- <script>alert(1)</script> --><p>ok</p>");
        assert!(!out.contains("alert"), "{out}");
        assert!(!out.contains("script"), "{out}");
        assert_eq!(out, "<p>ok</p>");
    }

    #[test]
    fn treats_entities_as_text_not_tags() {
        // `&lt;script&gt;` 只是文本，不能被当成标签执行。
        let out = sanitize_html("&lt;script&gt;alert(1)&lt;/script&gt;");
        assert!(!out.contains("<script"), "{out}");
        assert!(out.contains("&lt;script&gt;"), "{out}");
    }

    #[test]
    fn removes_template_and_noscript_content() {
        let out = sanitize_html(
            "<template><img src=x onerror=alert(1)></template><noscript><p>x</p></noscript>",
        );
        assert!(!out.contains("onerror"), "{out}");
        assert!(!out.contains("template"), "{out}");
        assert!(!out.contains("noscript"), "{out}");
    }

    #[test]
    fn removes_object_embed_applet_and_base_link_meta() {
        let out = sanitize_html(
            r#"<object data="x.swf"></object><embed src="x"><applet code="x"></applet><base href="http://evil.example/"><link rel="stylesheet" href="http://evil.example/x.css"><meta http-equiv="refresh" content="0;url=http://evil.example/">"#,
        );
        for tag in ["object", "embed", "applet", "base", "link", "meta"] {
            assert!(!out.contains(&format!("<{tag}")), "{out}");
        }
        assert!(!out.contains("evil.example"), "{out}");
    }

    #[test]
    fn drops_every_tag_from_the_drop_list() {
        // DROP_WITH_CONTENT 里的标签必须一个都不剩（连内容一起删）。
        for tag in DROP_WITH_CONTENT {
            let payload = format!("<{tag}>PAYLOAD</{tag}>");
            let out = sanitize_html(&payload);
            assert!(!out.to_lowercase().contains(&tag.to_lowercase()), "{out}");
            // 空元素（`embed` / `input` / `meta` …）本来就没有内容，
            // 只需要标签消失；其它标签必须连内容一起删。
            if !VOID_TAGS.contains(tag) {
                assert!(!out.contains("PAYLOAD"), "{tag} 的内容没有被删掉：{out}");
            }
        }
    }

    #[test]
    fn fixes_mismatched_tags() {
        let out = sanitize_html("<b><i>x</b></i>");
        // 输出必须良构：两个标签都闭合，且顺序正确。
        assert_eq!(out, "<b><i>x</i></b>");
    }

    #[test]
    fn drops_stray_end_tags() {
        let out = sanitize_html("</div></p>text");
        assert_eq!(out, "text");
    }

    #[test]
    fn closes_unclosed_elements_at_the_end() {
        let out = sanitize_html("<div><p>x");
        assert_eq!(out, "<div><p>x</p></div>");
    }

    // ── 良性样本 ────────────────────────────────────────────────────────

    #[test]
    fn keeps_paragraph_and_inline_formatting() {
        let out = sanitize_html("<p>Hello <b>world</b> and <i>others</i></p>");
        assert_eq!(out, "<p>Hello <b>world</b> and <i>others</i></p>");
    }

    #[test]
    fn keeps_lists() {
        let out = sanitize_html("<ul><li>a</li><li>b</li></ul><ol start=\"3\"><li>c</li></ol>");
        assert!(out.contains("<ul>"), "{out}");
        assert!(out.contains("<li>a</li>"), "{out}");
        assert!(out.contains("<ol start=\"3\">"), "{out}");
    }

    #[test]
    fn keeps_tables_with_spans() {
        let out = sanitize_html(
            r#"<table><thead><tr><th colspan="2">h</th></tr></thead><tbody><tr><td rowspan="2">1</td><td>2</td></tr></tbody></table>"#,
        );
        assert!(out.contains("<table>"), "{out}");
        assert!(out.contains("<thead>"), "{out}");
        assert!(out.contains(r#"<th colspan="2">h</th>"#), "{out}");
        assert!(out.contains(r#"<td rowspan="2">1</td>"#), "{out}");
        assert!(out.contains("<td>2</td>"), "{out}");
    }

    #[test]
    fn keeps_safe_links_with_title() {
        let out =
            sanitize_html(r#"<a href="https://example.com/x?a=1&amp;b=2" title="t">link</a>"#);
        assert!(
            out.contains(r#"href="https://example.com/x?a=1&amp;b=2""#),
            "{out}"
        );
        assert!(out.contains(r#"title="t""#), "{out}");
        assert!(out.contains(">link</a>"), "{out}");
    }

    #[test]
    fn keeps_mailto_links() {
        let out = sanitize_html(r#"<a href="mailto:a@b.com">mail</a>"#);
        assert!(out.contains(r#"href="mailto:a@b.com""#), "{out}");
    }

    #[test]
    fn keeps_blockquote_pre_code_and_headings() {
        let out = sanitize_html("<h1>T</h1><blockquote><pre><code>x=1</code></pre></blockquote>");
        assert!(out.contains("<h1>T</h1>"), "{out}");
        assert!(out.contains("<blockquote>"), "{out}");
        assert!(out.contains("<pre><code>x=1</code></pre>"), "{out}");
    }

    #[test]
    fn keeps_plain_text_and_escapes_specials() {
        let out = sanitize_html("a < b & c > d");
        assert_eq!(out, "a &lt; b &amp; c &gt; d");
    }

    #[test]
    fn strips_extra_link_attributes_but_keeps_the_link() {
        // 结构性断言：只留下 href，链接本身与文字都还在。
        let out = sanitize_html(
            r#"<a href="https://x.example/" target="_blank" rel="noopener" style="color:red" onclick="e()">t</a>"#,
        );
        assert_eq!(out, r#"<a href="https://x.example/">t</a>"#);
    }

    #[test]
    fn drops_out_of_range_span_attributes() {
        // 超大的 colspan 会被丢掉（避免排版引擎做无意义的计算）。
        let out = sanitize_html(r#"<table><tr><td colspan="2000000000">x</td></tr></table>"#);
        assert_eq!(out, "<table><tr><td>x</td></tr></table>");
        // 正常范围的要保留。
        let ok = sanitize_html(r#"<table><tr><td colspan="2">x</td></tr></table>"#);
        assert!(ok.contains(r#"colspan="2""#), "{ok}");
    }

    #[test]
    fn removes_img_but_keeps_surrounding_text() {
        let out = sanitize_html("<p>a<img src=\"https://track.example/p.gif\">b</p>");
        assert_eq!(out, "<p>ab</p>");
    }

    #[test]
    fn unwraps_unknown_tags_but_keeps_their_content() {
        // 未知标签**只删标签**，内容留着。
        //
        // 真实邮件的正文全都被 `html` / `body` 包着，
        // 连内容一起删的话整封邮件就没了。
        assert_eq!(sanitize_html("<font>x</font>"), "x");
        assert_eq!(sanitize_html("<center>x</center>"), "x");
        // 没闭合的未知标签也只影响标签本身。
        assert_eq!(sanitize_html("<o:p>x"), "x");
        // 整篇文档的壳子被剥掉，正文留下。
        assert_eq!(
            sanitize_html("<html><body><p>ok</p></body></html>"),
            "<p>ok</p>"
        );
    }

    #[test]
    fn keeps_content_of_a_dangerous_tag_out() {
        // 危险标签的**内容**必须一起删——它本身就是载荷。
        assert_eq!(sanitize_html("<script>alert(1)</script>"), "");
        assert_eq!(sanitize_html("<style>p{}</style>"), "");
        assert_eq!(sanitize_html("<iframe src=x>inner</iframe>"), "");
        // 外面的内容不受影响。
        assert_eq!(
            sanitize_html("<p>前</p><script>alert(1)</script><p>后</p>"),
            "<p>前</p><p>后</p>"
        );
    }

    #[test]
    fn drops_relative_and_protocol_relative_urls() {
        // 邮件正文里没有基地址，相对链接点了必然是坏的。
        for payload in [
            r#"<a href="/path">x</a>"#,
            r#"<a href="//example.com/x">x</a>"#,
            r#"<a href="page.html">x</a>"#,
        ] {
            let out = sanitize_html(payload);
            assert_eq!(out, "<a>x</a>", "{out}");
        }
    }

    #[test]
    fn attribute_value_cannot_break_out_of_the_quotes() {
        let out = sanitize_html(r#"<a href="x&quot; onclick=&quot;alert(1)">t</a>"#);
        assert!(!out.contains("onclick"), "{out}");
        assert!(!out.contains("alert"), "{out}");
        assert_eq!(out, "<a>t</a>");
    }

    #[test]
    fn handles_single_quoted_attributes_and_odd_spacing() {
        assert_eq!(
            sanitize_html("<a href='https://x.example/'>t</a>"),
            r#"<a href="https://x.example/">t</a>"#
        );
        // 结束尖括号前的空格。
        assert_eq!(sanitize_html("<p >x</p >"), "<p>x</p>");
    }

    #[test]
    fn keeps_text_around_removed_script() {
        let out = sanitize_html("before<script>bad()</script>after");
        assert_eq!(out, "beforeafter");
    }

    #[test]
    fn empty_input_gives_empty_output() {
        assert_eq!(sanitize_html(""), "");
    }

    #[test]
    fn allows_only_whitelisted_tags() {
        // 每个白名单标签都应该原样保留。
        for tag in [
            "p",
            "b",
            "strong",
            "i",
            "em",
            "u",
            "s",
            "ul",
            "ol",
            "li",
            "a",
            "blockquote",
            "code",
            "pre",
            "span",
            "div",
            "h1",
            "h2",
            "h3",
            "h4",
            "h5",
            "h6",
            "table",
            "thead",
            "tbody",
            "tr",
            "td",
            "th",
        ] {
            let out = sanitize_html(&format!("<{tag}>x</{tag}>"));
            assert_eq!(out, format!("<{tag}>x</{tag}>"), "标签 {tag} 应当保留");
        }
        for tag in ["br", "hr"] {
            let out = sanitize_html(&format!("<{tag}>"));
            assert_eq!(out, format!("<{tag}>"), "标签 {tag} 应当保留");
        }
    }

    #[test]
    fn never_emits_a_raw_lt_from_text() {
        // 输出里除标签本身以外不允许出现裸 `<`。
        let out = sanitize_html("<p>a < b</p><script>x</script>");
        let mut depth = 0i32;
        for ch in out.chars() {
            match ch {
                '<' => depth += 1,
                '>' => depth -= 1,
                _ => {}
            }
        }
        assert_eq!(depth, 0, "标签应当闭合：{out}");
    }

    // ── HTML → 纯文本 ────────────────────────────────────────────────────

    #[test]
    fn text_keeps_the_words_and_drops_the_tags() {
        let out = html_to_text("<p>今天开会</p><p>讨论进度</p>");
        assert_eq!(out, "今天开会\n讨论进度");
    }

    #[test]
    fn text_drops_script_and_style_content() {
        let out = html_to_text("<p>正文</p><script>alert(1)</script><style>p{color:red}</style>");
        assert_eq!(out, "正文");
    }

    #[test]
    fn text_decodes_entities() {
        let out = html_to_text("<p>&lt;标签&gt; &amp; 实体</p>");
        assert_eq!(out, "<标签> & 实体");
    }

    #[test]
    fn text_separates_table_cells_with_tabs() {
        let out =
            html_to_text("<table><tr><td>甲</td><td>乙</td></tr><tr><td>丙</td></tr></table>");
        assert_eq!(out, "甲\t乙\n丙");
    }

    #[test]
    fn text_collapses_whitespace() {
        let out = html_to_text("<p>  很多   空格  </p><p></p><p><br>换行</p>");
        assert_eq!(out, "很多 空格\n\n换行");
    }

    #[test]
    fn text_turns_nbsp_into_a_plain_space() {
        let out = html_to_text("<p>不\u{a0}断\u{a0}行</p>");
        assert_eq!(out, "不 断 行");
    }

    #[test]
    fn text_of_empty_input_is_empty() {
        assert_eq!(html_to_text(""), "");
    }

    #[test]
    fn text_of_a_plain_sentence_is_the_sentence() {
        assert_eq!(html_to_text("就一句话"), "就一句话");
    }
}
