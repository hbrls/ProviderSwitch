# 连通性检查 UI 调整方案

## 已澄清的约束

- 以原检测范围为准，不新增检查类型、状态或检测行为。
- 本次只调整 UI 与 UI 所需的结果传递方式；不改检测本身。
- 不新增、不修改测试，也不把测试作为本次实施工作。
- 原需求中的 “Model” 指 Modal 弹窗；卡片 Body 按字面始终显示 `Hello Model`。

## 目标交互

1. 点击首页一级“检查连通性”按钮，仅打开结果弹窗，不在按钮点击处理器中发起检查。
2. 弹窗挂载/打开后由 `useEffect` 发起现有的一次连通性检查。
3. 成功（原状态 `operational`）时不弹成功 toast；在弹窗 Card Title 显示原有成功文字（供应商名、耗时）。
4. Card Body 始终显示 `Hello Model`。
5. 慢速、不可达、异常仍按原有路径和原检测范围反馈，不额外扩展弹窗状态展示；原有 warning/error toast 行为保持不变。

## 当前实现与涉及位置

- `src/components/providers/ProviderCardActions.tsx`
  - 一级按钮已通过 `onTest` 显示。
  - 将按钮回调语义改为打开弹窗，不在此处直接运行检查；按钮的图标、禁用和检测行为以弹窗状态为准，不扩大可用范围。
- `src/components/providers/ProviderCard.tsx`
  - 持有完整 `provider` 与 `appId`，适合作为弹窗宿主；保留当前 `onTest` 存在、`appId !== "mcode"`、`provider.category !== "official"` 的显示条件。
  - 管理弹窗 open 状态，并将当前 Provider 传给弹窗。
- `src/components/providers/ProviderList.tsx`
  - 提供 `checkProvider(providerId, providerName)` 与 `isChecking(providerId)`。
  - 仍负责连接现有检查 hook，不改变检查目标、请求或其他结果处理。
- `src/hooks/useStreamCheck.ts`
  - 当前成功、慢速、失败和异常分别走既有 toast；API 返回 `StreamCheckResult | null`。
  - 仅为成功结果提供 UI 结果展示所需的回传/静默选项；默认行为维持不变，以免影响其他调用方。
- `src/lib/api/connectivity-check.ts`
  - 不修改。

## 结果范围边界

原有检测状态和反馈范围维持如下：

| 原检测结果 | 本次处理 |
| --- | --- |
| `operational` | 去掉该次成功 toast；把原有成功文案及耗时写入弹窗 Card Title |
| `degraded` | 保持原 warning toast，不新增弹窗结果文案 |
| `failed` | 保持原 error toast 与说明，不新增弹窗结果文案 |
| 检查抛错 / 返回 `null` | 保持原 error toast，不新增新的结果状态或说明 |

弹窗的 Card Body 不承载检测状态或结果，仅固定显示 `Hello Model`。检查进行期间不新增“检查中”等业务结果提示；可沿用现有 loading/disabled 视觉来防止重复触发，但不扩展检测语义。

## 组件结构建议

```text
ProviderCard
  ├─ ProviderCardActions
  │   └─ 检查连通性按钮：打开弹窗
  └─ ProviderConnectivityDialog（受控 open）
      └─ DialogContent
          ├─ DialogTitle：稳定的弹窗无障碍名称
          └─ Card
              ├─ CardHeader / CardTitle：成功时显示原成功文案；成功前可留空
              └─ CardContent：固定显示 Hello Model
```

弹窗在 ProviderCard 层管理，因为此层已经持有 Provider 与 appId。弹窗打开后在 effect 中执行一次检查；Provider 或打开周期变化时重置成功文案，避免上一个供应商的结果残留。关闭后异步结果不得写进另一个 Provider 的弹窗状态。检查 API 没有取消能力，不增加取消请求机制。

## Hook 适配原则

- 保留 `useStreamCheck` 当前检查请求、检测 ID、错误捕获和所有结果文案。
- 为弹窗调用增加局部的成功静默/结果回传能力；只让该弹窗调用抑制 `operational` 成功 toast。
- 其他调用者维持原默认 toast 行为。
- `degraded`、`failed`、异常继续按现有 hook 逻辑 toast；不得改成统一静默或重新定义状态。
- 不复制 `streamCheckProvider` 调用，不创建新的检查入口，不调整后端/API、超时或网络配置。

## Design Token 与组件规范

参考 `docs/design-system.html`、`src/index.css`、`tailwind.config.cjs`、`src/components/ui/dialog.tsx` 和 `src/components/ui/card.tsx`：

- 弹窗直接复用 `Dialog` / `DialogContent`。组件已提供居中布局、遮罩、`bg-surface`、`text-fg-1`、14px `rounded-dialog` 和 `shadow-v7-lg`。
- 内部结果卡复用 `Card`，使用卡片 surface、默认 border、panel/card 圆角与轻阴影，不增加新的 token 或颜色。
- `CardTitle` 默认 `text-2xl`（24px），对本弹窗里的紧凑状态卡偏大；覆盖为 `text-title`（16px/24px、600），使用 `text-fg-1`。
- 成功文字若需状态强调，使用 `text-success-text` token；保持文案本身不变，不额外添加图标、说明或状态标签。
- 正文使用 `text-body`（13px/20px）、`text-fg-1`。卡片 Header/Body 用紧凑 padding（约 16px），Body 始终只有 `Hello Model`。
- 弹窗宽度用现有尺寸档（建议 `max-w-md`）；不新增响应式布局规则，除非现有 DialogContent 默认约束无法容纳原有成功文案。
- DialogTitle 提供稳定的无障碍名称；结果 CardTitle 只承担卡片标题，不与 Radix DialogTitle 混用或互相覆盖。

## 实施步骤

1. 在 ProviderCard 增加弹窗开关状态，并把按钮动作改为打开弹窗；保留原供应商可用范围条件。
2. 新增 ProviderConnectivityDialog，复用项目 Dialog/Card primitives；打开后通过 effect 调用现有检查函数。
3. 将本次检查的 `operational` 文案回传给 Dialog，抑制这一次成功 toast，并在 CardTitle 显示；其他结果仍走原 toast。
4. CardContent 始终渲染字面文本 `Hello Model`。
5. 不改测试文件、不新增测试、不改变检测 API 或结果范围。实施完成后只做必要的静态核对（如 diff、类型或格式检查，若改动需要）。

## 不在本次范围

- 新增 loading、超时、重试、取消等检测行为。
- 把慢速、不可达或异常状态迁移到弹窗。
- 改动 warning/error toast、原有成功文案、检查接口、后端逻辑、网络策略或熔断器。
- 新增或修改任何测试。
