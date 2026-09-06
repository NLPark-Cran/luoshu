# 技术决策记录 002：跨会话记忆架构 MVP（2026-09-07）

## 结论（v1）

一张 sqlite 表 + 两个钩子，嵌入猹询码服务端：

1. **存储**：`memories(id, user_id, project_id NULL=user 级, kind[fact|preference|decision|gotcha], content, salience, evidence, source_session_id, archived, created_at, updated_at)` + FTS5 索引（v1 不上向量，语料小；后续加向量列）
2. **写入**：会话结束/压缩时跑一次门槛式抽取（gatekeeper → 候选抽取（带原文证据守卫）→ FTS top-k 相似 → LLM 判定 create/merge/conflict/skip；冲突归档不删除）——借鉴镜听空间 R9；另加显式 `remember` 工具
3. **注入**：worker 启动时取 top ~15 条（salience×recency），渲染为 Markdown 块附加进 system prompt（PenguinHarness 的 index 模式）；`search_memory` 工具按需深查
4. **人格**：静态 soul.md + 每用户私有 overlay（镜听模式）
5. **治理**：记忆 UI 只读+删除；凭证类内容永不入库（证据守卫 + denylist）

## 参考实现

- 镜听空间（here.crina.at）：gated extraction + verbatim-evidence guard（本地可读的生产代码）
- PenguinHarness：index eagerly, body on demand, model owns semantics
- Memorix：跨工具记忆（MCP 形态，后续对洛书/外部 agent 暴露时再参考）
- lingdong-code：人格/日程/关系层级
