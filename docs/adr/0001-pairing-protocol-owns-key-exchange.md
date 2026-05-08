# 配对协议负责双端密钥交换

Niuma 不保留旧配对协议兼容，新的唯一配对协议必须在 `/pair/confirm` 完成 iOS 与桌面端的长期加密公钥交换。`niuma-server` 只验证 pair token、双方签名和路由关系，不持久化双方 encryption public key，也不缓存离线 handshake；配对成功必须依赖在线 desktop agent 解密 handshake 并返回 signed ack。这样 server 继续保持 payload-blind 控制面，代价是配对必须在 desktop agent 在线时完成，失败后由用户刷新二维码重试。

**Status:** accepted

**Consequences:** 旧 QR `fingerprint` 语义、旧 `/pair/confirm` 只创建 binding 的路径、以及任何 v1/v2 分支都应删除。真实手机通过扫描桌面 loopback 页面展示的二维码完成配对，不直接访问本地 HTTP 服务。
