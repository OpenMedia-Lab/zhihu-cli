<p align="center"><a href="README.md">English</a> · <strong>简体中文</strong></p>

<p align="center"><img src="docs/assets/readme-hero.png" alt="zhihu-cli: write, review a draft, then publish" width="100%"></p>

# zhihu-cli

**在终端阅读知乎，把写好的内容保存成可审阅的文章草稿。** Rust 实现，支持 App 扫码登录、HTML 正文、图片、封面与话题。命令名为 `zhihu`。

[快速开始](#快速开始) · [草稿到发布](#草稿审阅发布) · [完整命令手册](docs/usage.md) · [English](README.md)

- **先读再写。** 搜索，读取文章、回答、问题、想法及账号历史。
- **先审阅真实草稿。** 创建文章默认保存远端草稿，显式传入 `--publish` 才公开。
- **修改同一篇文章。** 更新其草稿，保留未指定字段，再发布修订。

## 快速开始

需要当前稳定版 Rust 工具链。从源码安装：

```sh
git clone https://github.com/OpenMedia-Lab/zhihu-cli.git
cd zhihu-cli
cargo install --path . --locked

zhihu login --qr
zhihu whoami
zhihu search 'Transformer'
```

用知乎 App 扫码并在手机确认。加 `--qr-file ./zhihu-login.svg` 可导出私有二维码文件。也支持浏览器 Cookie 提取与标准输入导入，见[认证说明](docs/usage.md#auth)。

## 草稿、审阅、发布

将文章 HTML 写入 `body.html`。**下面会在知乎保存文章草稿：**

```sh
zhihu article '理解 Attention' --content-file body.html
zhihu draft ARTICLE_ID
```

使用返回的 ID 和 `edit_url` 检查内容。需要时使用自己的封面、图片和精确话题名称：

```sh
zhihu edit-article ARTICLE_ID --content-file revised.html
zhihu edit-article ARTICLE_ID --cover cover.png -t '深度学习'
```

**下面会公开发布文章或草稿中的修订：**

```sh
zhihu edit-article ARTICLE_ID --publish
```

未指定标题、正文、封面和话题时保留原值。指定话题会替换整组话题；未提供替换正文时，图片追加至原正文。已发布文章仍可有尚未发布的草稿修订，因此还要检查草稿本身。

## 按任务选择命令

| 任务 | 命令或指南 |
|---|---|
| 阅读文章、回答、问题、想法 | `zhihu read 'URL_OR_ID'` |
| 查看自己的文章 | `zhihu my-articles --limit 10` |
| 查询话题 | `zhihu topics '关键词'` |
| 转存图片 | `zhihu upload image.png` |
| 草稿、编辑、发布与删除 | [完整命令手册](docs/usage.md) |

**想法会立即公开发布。** 上述草稿流程只适用于文章。CLI 不提供类似小红书的私密笔记交付方式。图片处理可能滞后，发布前先回读草稿。

## 自动化与边界

运行错误以 JSON 写入 stderr 并返回非零退出码。多数成功命令返回结构化 JSON；不同操作的结构并不完全相同。平台会话、网页接口、审核与频率限制依然有效。

Cookie 保存在 `~/.config/zhihu-cli/cookies.json`，请勿提交到 Git。扫码确认与旧会话保护见[登录验证说明](docs/qr-login-verification.md)。扫码与想法的实测覆盖少于文章草稿流程。

## 参与贡献

提交问题时附上最小复现命令、预期行为和脱敏输出。修改代码后运行：

```sh
cargo fmt --check
cargo test --no-default-features
cargo check --locked
```

## 一起使用

[postdeck](https://github.com/OpenMedia-Lab/postdeck) 负责文章加工与图卡；平台操作交给独立 CLI： [xhs-cli](https://github.com/OpenMedia-Lab/xhs-cli), [zhihu-cli](https://github.com/OpenMedia-Lab/zhihu-cli), [x-cli](https://github.com/OpenMedia-Lab/x-cli). 更多项目见 [OpenMedia-Lab](https://github.com/OpenMedia-Lab).

## 许可

[MIT](LICENSE)。
