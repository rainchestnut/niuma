# Niuma iOS App

该目录用于承载 Niuma iOS 客户端工程。

## 测试安装

iOS app 目前仍处于测试阶段。需要安装到 iPhone 时，请通过 Xcode 打开
`niuma.xcodeproj`，选择真机和签名团队后自行编译运行；也可以发送邮件到
`rainchestnut@gmail.com` 获取 TestFlight 测试链接。

## 运行与构建

```bash
cd /Users/rainchestnut/research/openai/niuma/niuma-ios
xcodebuild -list -project "niuma.xcodeproj"
xcodebuild -scheme "niuma" -project "niuma.xcodeproj" -destination "platform=iOS Simulator,name=iPhone 17" build
```

应用入口是 `niuma/App/NiumaApp.swift`，依赖组装在
`niuma/App/AppContainer.swift`。当前 App 始终使用 `LiveNiumaController`
连接真实 server，移动端展示状态和同步 cursor 落在 SwiftData。

## 文件与图片同步

iOS 端协议只使用统一附件形态：

- `file_ref` 表示图片、视频和普通文件引用。
- `file_type` 表示宽类型：`image`、`video` 或 `file`。
- `transfer_id` 是完整 transfer payload 的 SHA-256，也是本地附件缓存表的主键。

发送附件时，iOS 先按内容生成 `transfer_id`，调用
`POST /transfers/{transfer_id}/ensure`，只有 `needs_upload=true` 时才用
`PUT /transfers/{transfer_id}` 上传完整 payload。发送后，原始附件保存到本地
附件目录，并在 SwiftData 中保存 `transfer_id -> localRelativePath` 映射。

接收桌面端附件时，iOS 先命中本地附件缓存；未命中时从 server 临时缓存下载，
校验 `sha256(data) == transfer_id` 后写入本地缓存并发送 ACK。移动端附件渲染只
依赖统一的 `file_ref`、`file_type` 和本地附件映射。
