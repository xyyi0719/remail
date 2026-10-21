# 环境特性与平台差异

> 用途：记录**本机与各目标平台**的实测特性，避免在脚本、CI、命令和文档中重复踩坑。
> 维护者：xyyi0719 · 最后实测：2026-10-21
> 原则：**只写实测过的**。未验证的写「未验证」，不写推测。
> **本机手动设置的值（代理地址、凭据、密钥路径等）不写进本文档**，只写读取方法。

---

## 1. 本机开发环境（Windows）

### 1.1 工具链实测版本

| 工具 | 版本 | 备注 |
|---|---|---|
| rustc / cargo | 1.99.1 | 满足 `rust-version = 1.88` |
| Node.js | 22.21.0 | |
| npm | 随 Node | `package-lock.json` 已提交 |
| Python | 3.11.0 | CI 的 docs job 用它跑脚本 |
| OpenJDK | **仅 1.8.0_502** | ⚠️ **无 JDK 17，无 Android SDK/NDK** |
| gh CLI | 2.102.0 | 已登录 `xyyi0719` |
| Git | 内置 | `filter-repo` **未安装**（只能用 `filter-branch`） |

> **Android 构建在本机暂不可行**：需要 JDK 17 + Android SDK + NDK。见 [ROADMAP P4](ROADMAP.md)。

### 1.2 Shell 差异（最容易出错的地方）

**Windows 默认 shell 是 PowerShell，不是 bash。**

| 语法 | PowerShell | bash | 备注 |
|---|---|---|---|
| `$(cmd)` | ❌ 不支持（是数组字面量） | ✅ | **CI Windows job 必须显式 `shell: bash`** |
| `grep` | ❌ 需装 Git Bash 或用 `Select-String` | ✅ | |
| `2>/dev/null` | ❌ | ✅ | |
| `\|\|` 兜底 | ✅ | ✅ | |
| `cmd1 && cmd2` | ✅（新版） | ✅ | |

> **实例（真实踩坑）**：CI 的用例计数步骤最初写成 bash 语法，Ubuntu job 通过、
> **Windows job 失败**：
> ```
> The term 'count=$(cargo test ...)' is not recognized
> ```
> 修法：`shell: bash`，并在本机 Git Bash 下先验证命令输出。

**在本机执行 bash 语法命令**：用 `bash -c '...'` 显式调用，不要假设默认 shell。

### 1.3 命令行的多层引号陷阱

`ctx_execute` / `bash` 工具会把命令再交给外层 shell（cmd 或 sh），
命令**自身**的引号会被再解释一层。

| 坑 | 现象 | 正确做法 |
|---|---|---|
| `curl -w "%{http_code}"` | `%{` 被外层展开 → 输出 `%{http_code}` 字面量 | 用 `\n__CODE__%{http_code}` 之类**无歧义标记**，再正则提取 |
| `grep -c ": test$"` | `$` 被外层吞掉 | 单引号包裹整个 pattern |
| 命令里含 `'`（如 `don't`） | 提前闭合引号 | 改用双引号或转义 |
| `cmd` 中的 `&` `|` `>` | 被当作管道/重定向 | 引号包裹整条命令 |

> **原则**：需要精确控制时，**用脚本文件**（写入临时 `.py`/`.js` 再执行），
> 不要把复杂逻辑塞进单行命令字符串。

### 1.4 不可用的命令

| 命令 | 原因 |
|---|---|
| `timeout`（GNU coreutils） | 本机无此命令；`timeout /t 10` 在非交互 shell 下报 exit 125 |
| `grep`（纯 PowerShell 环境） | 见 §1.2 |
| `git filter-repo` | 未安装 |
| `sed -i` | Windows 上行为与 GNU 有差异，改用 Node 脚本改文件 |

### 1.5 网络与代理（重要）

**系统代理由维护者手动设置在 Windows 系统设置里，地址随网络环境变化，本文不写死具体地址。**
用之前先读**当前值**：

```bash
# Git Bash / MSYS 下 /v 会被当成路径改写，用 //v 或 MSYS_NO_PATHCONV=1
reg query 'HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings' //v ProxyServer
reg query 'HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings' //v ProxyEnable
reg query 'HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings' //v ProxyOverride
```

| 注册表值 | 含义 |
|---|---|
| `ProxyEnable` | `1` = 代理开启，`0` = 关闭 |
| `ProxyServer` | 当前代理 `host:port`（HTTP 代理） |
| `ProxyOverride` | **绕过代理**的主机列表；本机实测含 `localhost`、`127.*`、`10.*`、`172.16.*`–`172.31.*`、`192.168.*`、`*.cn`、`*.local`、`*.cloud`、`<local>` → 内网地址与 `*.cn` 走直连 |

**仓库级 `http.proxy` / `https.proxy` 同样是手动写的**：落在 `.git/config`，
而 `.git/` 不被 git 跟踪，因此**不会随仓库分发**，换机器必须重设。
查看当前值：`git config --local --get http.proxy`。

**关键差异**：

| 工具 | 是否自动使用系统代理 | 说明 |
|---|---|---|
| `gh` | ✅ 会 | Go 运行时读取 Windows 代理设置 |
| `git` | ❌ 不会 | 必须显式 `git config http.proxy` |
| `curl` | ❌ 不会 | 必须显式 `curl -x` |
| Node `fetch` | ❌ 不会 | 需要显式代理 agent |

**DNS 劫持**：`github.com` 在本机解析到 **127.0.0.1**（企业网络策略），而 `api.github.com` 正常。

| 目标 | 直连 | 走代理 | 绕行方式 |
|---|---|---|---|
| `api.github.com` | ✅ 200 | ✅ 200 | — |
| `github.com` | ❌ 解析到 127.0.0.1 | ✅ 200 | 走代理，或 `--resolve` 配实时 IP |
| `github.com:22`（SSH） | ❌ 连接被拒 | — | 改用 **SSH over 443**：`ssh://git@ssh.github.com:443/...` |
| `developers.google.com` | ❌ 连接超时 | — | 无法核实 Google 官方文档，相关结论须标注未核实 |
| `crates.io` / `npm registry` | ✅ 可达 | — | 依赖版本可实时核对 |

**`--resolve` 用的 GitHub 公网 IP 会轮换，不要写死**。需要时经代理走 DoH 实时取：

```bash
PXY=$(reg query 'HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings' //v ProxyServer \
      | tr -d '\r' | sed -n 's/^.*REG_SZ[ ]\{1,\}\(.*\)$/\1/p')
curl -sS -x "http://$PXY" -H 'accept: application/dns-json' \
     'https://dns.google/resolve?name=github.com&type=A'
```

**SSH 密钥、`.ssh/config` 主机别名、`.git/config` 的 `core.sshCommand` 与 remote URL 都是本机手动配置，不入库。**
本仓库 remote 走 **SSH over 443**（`ssh://git@ssh.github.com:443/...`），因为 22 端口在本机被拒。

**已知限制**：`gh auth refresh` 需访问 `github.com`（被劫持），**无法自助扩权**。
需要新 scope（如 `delete_repo`）时只能：
1. 手动改 hosts（**本机无写权限**，EPERM）
2. 用 `curl --resolve` 走设备流（已验证可行，但需人工在浏览器输入设备码）

#### 1.5.1 连 GitHub 前先探明

**每次要访问 GitHub（推送、拉取、调 API、开 PR）之前，先探明能不能连通，再执行操作。**
本机 `github.com` 被 DNS 劫持，直连必失败，很容易误判成「没网」或「仓库不存在」。

```bash
# 1) 推送走的真实通道（SSH over 443）——最贴近 push 本身
git ls-remote origin

# 2) API 通道（gh 自动读系统代理）
gh auth status
gh api rate_limit --jq '.rate.remaining'

# 3) HTTPS 通道（需显式 -x 走系统代理）
PXY=$(reg query 'HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings' //v ProxyServer \
      | tr -d '\r' | sed -n 's/^.*REG_SZ[ ]\{1,\}\(.*\)$/\1/p')
curl -sS -o /dev/null -w 'code=%{http_code}\n' -x "http://$PXY" https://api.github.com
```

判读：

| 探针结果 | 结论 | 下一步 |
|---|---|---|
| `git ls-remote` 成功 | 推送通道可用 | 可以 `git push` |
| `git ls-remote` 失败但 `gh api` 成功 | 仅 HTTPS / API 通道可用 | 检查 remote 是否为 SSH over 443 |
| 全部失败 | 代理未开或网络不通 | 读注册表 `ProxyEnable` / `ProxyServer` 当前值 |
| `curl` 报 `Unsupported proxy syntax` | 提取代理的管道写错了 | 核对 `PXY` 提取结果是否为空 |

**本机到 GitHub 的连接是间歇性的（2026-10-21 实测）**：

连续 5 次采样，同一批命令的结果在**成功与失败之间跳变**：

| 探针 | 5 次中的成功次数 | 失败时的现象 |
|---|---|---|
| `git ls-remote origin` | 3/5 | `Connection reset by <ip> port 443` |
| `gh api rate_limit` | **5/5** | — |
| `curl -x <代理> https://api.github.com` | 1/5 | `schannel: failed to receive handshake` |

→ **单次探针失败不等于断网**。先重试再下结论；`gh` 走 Go 运行时代理、最稳，
`curl`（Windows schannel）过企业代理最容易 TLS 握手失败，**不要把它当唯一判据**。

**三种失败分属不同通道，别混判（2026-10-21 逐条定位）**：

| 通道 | 失败信息 | 含义 |
|---|---|---|
| SSH over 443（推送走这条） | `Connection reset by <ip> port 443` + `fatal: Could not read from remote repository.` | **TCP 443 能连上，SSH 会话被中途 reset**；与证书、密钥、仓库权限都无关 |
| `gh api`（HTTPS / 代理） | `net/http: TLS handshake timeout` | Go 运行时与企业代理之间的 TLS 握手超时 |
| `curl`（HTTPS / 代理） | `schannel: failed to receive handshake, SSL/TLS connection failed` | Windows schannel 握手失败，**最不稳，不可作唯一判据** |

**遇到 SSH 的 `Connection reset` 不要换 GitHub IP**：2026-10-21 实测 9 个候选
（`20.205.243.160`、`20.205.243.166`、`20.27.177.113`、`20.200.245.247`、
`20.207.73.82`、`20.248.137.48`、`140.82.112.3`、`140.82.113.3`、`140.82.114.3`）：

- TCP 443 对**所有**候选都通 → 问题不在路由，在会话层；
- 但**只有本地 DNS 解析到的那个**接受 `ssh.github.com` 的 SSH 服务，其余全部
  `Connection closed by <ip> port 443`。

→ **换 IP 无效，只能重试。** 候选 IP 也会轮换，不要写进脚本。

**探不通就不要 push**：连接失败时反复推送只会产生半截状态或让 CI 空跑。
先按上表定位到具体通道，修好再推，并把实际探针输出当作证据记录。

**探针与 push 必须放在同一个重试循环里**：探针通过之后，`push` 自身仍可能在传输中被 reset。
2026-10-21 的一次真实推送，探针在前 3 次全失败、**第 4 次**才通过 —— `TRIES=3` 不够用。

**判成功只看 `git push` 自己的退出码**（2026-10-21 实测踩坑）：

- **不要给 `git push` 接管道**。`git push origin main 2>&1 | tail -3` 的退出码是 `tail` 的，
  永远是 0 —— 循环会把失败读成成功，把成功读成失败（实测一次已成功推送被误报为失败并重试了 40 次）。
  要保留输出就**重定向到文件**，别用管道。
- **推送后用 `ls-remote` 核验，本身也会间歇性失败**。核验失败 **不等于** 没推上去；
  以 `git push` 的退出码为准，核验要带重试。
- 核验要**比对 SHA**，不要只看命令是否报错：

```bash
# 正确的核验：比对远端 main 与本地 HEAD
for i in $(seq 1 8); do
  RSHA=$(timeout 30 git ls-remote origin refs/heads/main 2>/dev/null | cut -f1)
  [ -n "$RSHA" ] && break
  sleep 3
done
[ "$RSHA" = "$(git rev-parse HEAD)" ] && echo "远端已同步" || echo "远端未同步"
```

建议用带重试的探针 + 推送脚本（推送前跑一次，通过即推）：

```bash
#!/usr/bin/env bash
# GitHub 连通性探针 + 推送：本机连接间歇性失败，必须重试。
# 探针和 push 放在同一循环：探针过了 push 也可能被 reset。
TRIES=12
for i in $(seq 1 $TRIES); do
  if git ls-remote origin >/dev/null 2>&1; then
    echo "[$i/$TRIES] 探针 OK -> 执行 push"
    if git push origin main; then
      echo "PUSH_OK"; exit 0
    fi
    echo "push 失败（可能传输中被 reset），继续重试…"
  else
    echo "[$i/$TRIES] 探针失败（连接被 reset），3 秒后重试…"
  fi
  sleep 3
done
echo "PUSH_FAIL: $TRIES 次尝试内未完成推送"; exit 1
```

> 2026-10-21 实测：该脚本把一次真实推送送到了远端（第 4 次尝试通过），
> 随后 CI run `39559605711` 的 4 个 job 全绿。

### 1.6 文件系统

| 项 | 情况 |
|---|---|
| `C:\Windows\System32\drivers\etc\hosts` | **只读**（EPERM），无法加 hosts 条目 |
| 行尾 | `.gitattributes` 统一 `text=auto eol=lf`；Windows 上 `git add` 会提示 CRLF→LF 转换（正常） |
| 路径分隔符 | Rust 用 `std::path::Path`；文档链接用 `/`；**代码注释里的相对路径按 crate 位置算层级**（见下） |

### 1.7 代码注释里的文档相对路径

路径是**相对于源文件所在目录**的，不是相对于仓库根。

| 文件 | 正确层级 |
|---|---|
| `crates/xremail-core/src/*.rs` | `../../../docs/X.md`（src → crate → crates → 根） |
| `apps/shell/src-tauri/src/*.rs` | `../../../../docs/X.md`（src → src-tauri → shell → apps → 根） |

> 曾在 4 个文件里全部写成少一层，被 CI 的 doc-comment 检查抓出。
> **CI 已加自动检查**（见 `.github/workflows/ci.yml` 的 docs job）。

---

## 2. 目标平台差异

### 2.1 平台支持状态（截至 2026-10-20）

| 平台 | 状态 | 证据 |
|---|---|---|
| **Windows** | ✅ 可编译可运行 | 本机构建通过；CI `windows-latest` 绿 |
| **Linux (Ubuntu)** | ✅ **可编译**（运行未验证） | CI `ubuntu-latest` 绿，且 ubuntu job 编译了 tauri 应用本体 |
| **Android** | ⛔ 本机不可构建 | 缺 JDK 17 + SDK/NDK |
| **macOS** | ❓ 未验证 | 本机无 macOS；CI 未覆盖 |
| **iOS** | ❓ 未验证 | 需要 Apple 开发者账号 |

> Linux 编译是 **CI 实际证据**，不是推断：workspace 含 `apps/shell/src-tauri`，
> ubuntu job 执行 `cargo clippy/test --workspace`，且预装了 `libwebkit2gtk-4.1-dev`。

### 2.2 Linux 构建依赖

```bash
sudo apt-get install -y libwebkit2gtk-4.1-dev librsvg2-dev patchelf
sudo apt-get install -y libayatana-appindicator3-dev \
  || sudo apt-get install -y libappindicator3-dev
```

| 包 | 用途 | 缺失后果 |
|---|---|---|
| `libwebkit2gtk-4.1-dev` | WebView（注意是 **4.1**，不是 4.0） | 编译失败 |
| `librsvg2-dev` | 图标 | 编译失败 |
| `patchelf` | 修补动态库路径 | 打包失败 |
| `libayatana-appindicator3-dev` | 托盘图标 | 托盘不可用（可退化） |

### 2.3 各平台的 WebView 差异（**最重要的未验证项**）

| 平台 | 引擎 | 版本 | 对本项目的影响 |
|---|---|---|---|
| Windows | **Chromium**（WebView2） | 跟随 Edge | 安全模型的设计基准 |
| Android | **Chromium**（WebView） | 随系统更新，碎片化 | 需真机矩阵 |
| Linux | **WebKitGTK** | 跟随发行版 | ⚠️ **与 Chromium 行为不同** |
| macOS | **WebKit** | 跟随系统 | ⚠️ 同上 |

**为什么这件事重要**：我们的 HTML 邮件安全模型（[SECURITY §3](SECURITY.md)）依赖
sandbox iframe + CSP + `srcdoc`。这些行为在 **WebKitGTK 上与 Chromium 存在差异**：

- `sandbox` 属性对 `allow-popups` / `allow-popups-to-escape-sandbox` 的处理
- `srcdoc` 内联文档的 CSP 继承与 `meta` CSP 生效顺序
- `default-src 'none'` 能否真正阻断远程图片请求
- `blob:` / `data:` 在 opaque origin 下的可用性
- 外部链接在无 `allow-scripts` 下的可点击性

→ **结论**：Windows/Android 上验证过的沙箱行为，**不能直接推断到 Linux/macOS**。
在 WebKit 系平台上线前，必须重跑 [TESTING_STRATEGY §2.4](TESTING_STRATEGY.md) 的
网络请求计数、脚本执行、导航与 IPC 隔离测试。

### 2.4 平台相关 API 差异

| 能力 | Windows | Linux | macOS | Android |
|---|---|---|---|---|
| 系统凭据库 | Credential Manager | **Secret Service**（gnome-keyring/kwallet） | Keychain | **Keystore**（需自写适配） |
| 托盘 | 原生 | libayatana-appindicator | 原生 | 不适用 |
| 通知 | 原生 | **D-Bus / libnotify** | 原生 | NotificationManager |
| 开机自启 | Registry / Startup | **XDG autostart .desktop** | LaunchAgent | Background 限制 |
| 应用数据目录 | `%APPDATA%` | `~/.local/share` | `~/Library/Application Support` | app 内部存储 |

> `keyring` crate 4.x **没有按平台划分的 feature**（平台后端已拆为独立 crate），
> 且**不能假定自动支持 Android**。见 [SECURITY §2.1](SECURITY.md)。

### 2.5 前端无平台分支

当前 `apps/shell/src/App.tsx` **没有任何 OS 判断**（无 `navigator.platform` /
`userAgent` 分支）。所有平台差异都落在 Rust 侧与 Tauri 插件上。

→ 这是一件好事：平台差异集中处理，不必在前端散落 `if (isWindows)`。

---

## 3. 写脚本 / CI 前的检查清单

- [ ] 命令在 **PowerShell** 下能跑吗？还是要 `shell: bash`？
- [ ] 用到的命令（`grep` / `sed -i` / `timeout`）在目标 shell 存在吗？
- [ ] 引号能扛住外层 shell 再解释一层吗？复杂逻辑改用脚本文件
- [ ] 需要访问的网络目标在**目标环境**可达吗（代理 / DNS / 端口）？
- [ ] 要访问 GitHub 吗？**先探明连通性再操作**，别直接 push（见 §1.5.1）
- [ ] 依赖的本地工具装了吗（`filter-repo` / JDK 17 / SDK）？
- [ ] 文档相对路径的层级对吗（见 §1.7）？
- [ ] 平台差异是否影响这条命令/这段代码（见 §2.3）？

## 4. 本文档的维护

- 每次遇到新的环境坑 → 补进 §1，并在 CHANGELOG 记录
- 每次实测新的平台能力 → 更新 §2 的状态表（区分「可编译」与「可运行」）
- 推断性的内容一律标注「未验证」，不写成结论

相关：[架构](ARCHITECTURE.md) · [安全](SECURITY.md) · [测试策略](TESTING_STRATEGY.md) · [AI 规范](AI_DEVELOPMENT_GUIDE.md)
