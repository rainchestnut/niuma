# APNs 进度通知只做加密唤醒

Niuma 需要在 iOS 后台、锁屏或离线后提醒用户 Codex 任务已有新进度，因此服务端开始实现 APNs / Push Service；但 `niuma-server` 仍保持 payload-blind，只发送通用提醒文案和 Gateway 生成的端到端加密密文。移动端点击通知后解密密文中的 `session_id` / `thread_id`，再通过现有 `resume_thread` 链路刷新详情；过程 `task_update` 不能触发这类通知，只有 Codex turn 终态才算可通知的新进度。

**Status:** accepted
