# Cloudflare 下载与更新分发

地址：<https://wocli.yhan.fun/>。Worker 名称 `wocli-dist`，绑定 `yhan.fun` 区域的 `wocli.yhan.fun` 自定义域；Cloudflare 自动管理 DNS 与 HTTPS 证书。

用户访问 Cloudflare，Worker 在服务端读取固定的 `yhan-sun/wocli-music` GitHub Release，跟随安装包的签名下载跳转并直接返回文件。用户无需连接 GitHub。新版 Release 发布后最多约 10 分钟更新版本信息；安装包和源码包缓存 24 小时。发布时不要覆盖已有版本文件，修复应使用新版本号。

## 用户入口

```powershell
irm https://wocli.yhan.fun/install.ps1 | iex
```

```sh
curl -fsSL https://wocli.yhan.fun/install.sh | bash
```

新客户端的 `wocli update` 默认使用此分发服务。旧客户端仍然直连 GitHub，遇到连接问题可重新运行安装命令；安装器会停止旧后台并保留曲库。也可从网页下载后手动覆盖。

环境变量 `WOCLI_DOWNLOAD_BASE` 同时控制安装器和新版自更新：默认 `https://wocli.yhan.fun`；设置为 `github` 可直连 GitHub；也支持相同路径协议的 HTTPS 镜像地址。`update --direct` 只控制是否使用系统代理，不改变分发地址。

## 接口

- `/`：下载与安装说明。
- `/install.sh`、`/install.ps1`：打包进 Worker 的安装脚本文本，HTTP 响应为无 BOM 的 UTF-8 并声明字符集，兼容 `irm | iex`。仓库中的 PowerShell 文件保留 BOM，兼容 Windows PowerShell 5.1 的本地文件执行。
- `/api/releases/latest`：稳定版本信息，包地址全部指向自己的域名。
- `/download/<platform>`：跳转至本站最新稳定版安装包；`?sources=1` 下载对应源码。
- `/releases/download/v<version>/<asset>`：固定版本安装包、对应源码和 `SHA256SUMS`。

平台：`windows-x86_64`、`macos-arm64`、`macos-x86_64`、`linux-x86_64`。其余路径拒绝访问，不提供任意 URL 代理。仅接受 GET / HEAD，浏览器 Cookie、Authorization 不转发至上游；错误不会进入缓存。

安装器与自更新仍验证 SHA256。SHA256 用于文件完整性检查，不是数字签名。

## 维护与部署

普通发版沿用 GitHub Release 工作流，无需 Cloudflare Token，也无需再上传安装包。分发服务依赖 GitHub 上游；它是缓存分发，不是永久离线镜像。

修改 Worker 或安装脚本后重新打包、验证并部署：

```sh
python3 cloudflare/build.py
node --test cloudflare/worker.test.mjs
npx wrangler deploy --config cloudflare/wrangler.toml
```

也可在 Cloudflare 控制台 `Workers 和 Pages → wocli-dist → 编辑代码` 将生成的 `dist/cloudflare-worker.mjs` 完整替换为 Worker 代码后部署。生成文件包含两份安装器，勿只复制 `cloudflare/worker.mjs`。脚本变更需要重新部署 Worker；仅发布新版本无需重新部署。

部署后核对版本 API、四个平台包与源码链接、SHA256、安装器 BOM，并在隔离目录执行安装和 `update --check`。控制台支持回滚到之前的 Worker 部署版本。

本方案无需开通 R2 或付费计划；实际配额以 Cloudflare 控制台为准。普通全球网络不能保证中国大陆所有运营商可达或高速；真正的 Cloudflare China Network 是另行开通的服务。若后续需要更稳定的大陆分发，可在相同域名路径下切换至国内对象存储/CDN，客户端接口保持一致。

官方文档：[自定义域](https://developers.cloudflare.com/workers/configuration/routing/custom-domains/)、[Workers 配额](https://developers.cloudflare.com/workers/platform/limits/)、[China Network](https://developers.cloudflare.com/china-network/)。
