# Helix 25.07.1 之后的功能调查

调查日期：2026-11-02。

## 范围与方法

- 官方 [latest release](https://github.com/helix-editor/helix/releases/latest) 实际重定向至 [25.07.1](https://github.com/helix-editor/helix/releases/tag/25.07.1)，发布日期为 2025-07-18；日期亦见仓库 `CHANGELOG.md`。
- 已直接获取官方 master 和 25.07.1 tag。正式版对应提交为 `a05c151bb6e8e9c65ec390b0ae2afe7a5efd619b`；本地 HEAD 和上游 master 均为 `ba40e547`（2026-09-29）。
- `git rev-list --count research-upstream/release-25.07.1..research-upstream/master` 返回 989。这包含修复、语言资源、依赖和文档，不能理解为 989 个新功能。
- 以下是正式发布之后已进入 master 的主要变化，不是正式发行说明，也不覆盖尚未合并的 PR。

## 主要可用变化

| 方向 | 变化与实际意义 | 一手来源 |
| --- | --- | --- |
| 单词补全 | 从已打开 buffers 提取单词，无需 LSP；默认开启，默认输入 7 个单词字符后触发，可配置 trigger-length | [#13206](https://github.com/helix-editor/helix/pull/13206)、`book/src/editor.md` 的 word-completion 一节 |
| 无 LSP 符号导航 | 基于 Tree-sitter `tags.scm` 的文档/工作区符号 picker；默认 Space s/S 可选择 LSP 或语法信息，取决于语言支持 | [727758e0](https://github.com/helix-editor/helix/commit/727758e0)、`helix-term/src/keymap/default.rs` |
| 彩虹括号 | 基于语法树为嵌套括号着色；需开启 rainbow-brackets，且语言需包含 rainbows.scm；picker 预览也支持 | [#13530](https://github.com/helix-editor/helix/pull/13530)、[7ae2070b](https://github.com/helix-editor/helix/commit/7ae2070b)、`book/src/editor.md` |
| 保存自动修复 | code-actions-on-save 按顺序运行 LSP actions，可整理 imports、执行 fixAll；依赖服务器能力。Go 默认配置已启用 source.organizeImports | [#14481](https://github.com/helix-editor/helix/pull/14481)、`book/src/languages.md`、`languages.toml` |
| Code action 提示 | 可在 gutter 或 statusline 显示当前选择是否有可用 action | [#15820](https://github.com/helix-editor/helix/pull/15820)、`book/src/editor.md` |
| LSP 诊断 | 新增 pull diagnostics 协议支持；inline diagnostics 改为默认开启，诊断 picker 按严重程度排序 | [#11315](https://github.com/helix-editor/helix/pull/11315)、[#13970](https://github.com/helix-editor/helix/pull/13970)、[#13806](https://github.com/helix-editor/helix/pull/13806) |
| LSP 高亮与链接 | 可自动高亮光标处符号引用（auto-document-highlight 默认 false）；支持 documentLink 高亮、解析与打开 | [#15221](https://github.com/helix-editor/helix/pull/15221)、[#15235](https://github.com/helix-editor/helix/pull/15235)、[#15440](https://github.com/helix-editor/helix/pull/15440)、`helix-term/src/commands.rs` |
| LSP 文件操作 | 新增创建与删除文件操作的协议支持 | [#15448](https://github.com/helix-editor/helix/pull/15448) |
| 混合语言 | 缩进、textobjects、注释与续接注释、auto-pairs 按注入语言工作；例如 Markdown 代码块、Svelte 内 script/style | [64674f57](https://github.com/helix-editor/helix/commit/64674f57)、[d27856b0](https://github.com/helix-editor/helix/commit/d27856b0)、[5590a21a](https://github.com/helix-editor/helix/commit/5590a21a)、[#14697](https://github.com/helix-editor/helix/pull/14697) |
| 缩进引擎 | 重写为计算包含当前行的缩进作用域，并加入 corpus 检查工具；这是正确性与维护性改进，未据此声称全面性能提升 | [3013c107](https://github.com/helix-editor/helix/commit/3013c107)、[590f54e3](https://github.com/helix-editor/helix/commit/590f54e3) |
| 工作区信任 | 默认允许 LSP/DAP，项目本地配置和 Git full trust 需显式信任；支持授权、撤销，以及 .helix 配置变化后的校验 | [#15177](https://github.com/helix-editor/helix/pull/15177)、[#15857](https://github.com/helix-editor/helix/pull/15857)、`book/src/workspace-trust.md` |
| 主题和终端 | 深浅色主题随支持 mode 2031 的终端切换；更换主题时同步终端背景；新增 Kitty Keyboard Protocol 开关。非 Windows 默认终端后端迁移至 Termina，Windows 保留 Crossterm | [#14356](https://github.com/helix-editor/helix/pull/14356)、[c1040651](https://github.com/helix-editor/helix/commit/c1040651)、[#14398](https://github.com/helix-editor/helix/pull/14398)、[83abbe56](https://github.com/helix-editor/helix/commit/83abbe56)、[#14454](https://github.com/helix-editor/helix/pull/14454) |
| 文件和目录操作 | 原有 file explorer 增加选项与单子目录折叠；新增 pushd/popd；move! 可自动建立父目录 | [#13888](https://github.com/helix-editor/helix/pull/13888)、[#14173](https://github.com/helix-editor/helix/pull/14173)、[#14932](https://github.com/helix-editor/helix/pull/14932)、[#15001](https://github.com/helix-editor/helix/pull/15001) |
| 命令扩展 | 新增 set-register、%reg{}，以及绝对文件路径、工作区目录、当前工作目录展开 | [#14779](https://github.com/helix-editor/helix/pull/14779)、[#15410](https://github.com/helix-editor/helix/pull/15410)、[#12989](https://github.com/helix-editor/helix/pull/12989)、[#13068](https://github.com/helix-editor/helix/pull/13068) |
| 语言生态 | 新增 MoonBit、Puppet、Batch、C3 等语言；加入 vtsls、oxlint、Roslyn、pytest-language-server 等配置；有大量语法库与查询更新 | [#15700](https://github.com/helix-editor/helix/pull/15700)、[dfdc98a6](https://github.com/helix-editor/helix/commit/dfdc98a6)、[fead45db](https://github.com/helix-editor/helix/commit/fead45db)、[#15206](https://github.com/helix-editor/helix/pull/15206)、[#16051](https://github.com/helix-editor/helix/pull/16051)、[#14981](https://github.com/helix-editor/helix/pull/14981)、[#15259](https://github.com/helix-editor/helix/pull/15259)、[#15269](https://github.com/helix-editor/helix/pull/15269) |

## 避免误计

- File explorer、EditorConfig、LSP document colors、命令行 expansions、高亮引擎 tree-house 在 25.07 已有，不能作为这次发布间隔内首次新增的功能；来源：`CHANGELOG.md` 的 25.07 一节。
- Inline diagnostics 是默认配置变化，并非这段时间才首次实现。
- Call hierarchy 已合入协议底层，但 [ca2c4b51](https://github.com/helix-editor/helix/commit/ca2c4b51) 明确注明尚未通过命令暴露；当前 commands 和 keymap 也未找到调用层级命令。不能列为已可用的调用关系浏览功能。
- 当前 workspace 和 helix-term 的 Cargo 配置中未见插件运行时接入；本文不将外部插件开发进展算作本分支已交付功能。

## 核查限制

没有编译或交互测试以上功能。结论来自已合入上游的实现、文档、提交信息及官方 release 页面；LSP 功能还需相应 language server 提供支持。GitHub REST latest-release 请求返回 403，改用官方网页重定向确认正式版。
