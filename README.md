# Lamar Pet

一个把今日待办放在桌面上的 Windows 像素桌宠。

<img src="assets/lamar-cat.png" alt="Lamar Pet 角色动作图" width="360" />

## 功能

- 宠物下方常驻待办便签：最多显示三项，当前任务、截止与逾期清楚可见，支持收起。
- 直接勾选完成，六秒内可撤销；完成后显示下一件建议处理的任务。
- 点击任务标题或编辑图标，在原位置修改；Enter 保存、Esc 取消，新增和编辑草稿互不覆盖。
- 支持快速新增：输入 `15:30 发送周报`，自动拆出任务标题和当天截止时间。
- 常用按钮有悬停和按下反馈；编辑展开、保存、完成与收起有短动画。
- 桌面便签直接完成、撤销、开始或暂停专注；添加按钮直接定位新增输入框。
- 昨日未完成事项会保留；绑定任务的专注轮自然结束时自动勾选该任务，手动停止或暂停不会。
- 偏好设置可关闭桌面便签；任务随原 focus.json 保存，旧专注记录继续保留。

- 桌宠眨眼、呼吸、键鼠互动，休息时睡觉，计时完成时庆祝。
- 左键拖到任意位置停留，支持跨显示器拖动；轻点一下猫猫会开心回应，右键打开面板，大小可调整为 50%–200%。
- 面板打开时宠物自动让到面板之下，关闭后恢复置顶。
- 面板底部显示数据目录，一键打开所在文件夹。
- 专注开始时弹出提示，数秒后自动消失。
- 默认专注 25 分钟、休息 5 分钟，可直接输入时长，支持暂停、继续和提前结束。
- 专注结束进入休息；可开启"自动连转"，休息结束自动开始下一轮。
- 提醒窗口带宠物动画并播放提示音，声音可在设置中开关。
- 久坐提醒默认 45 分钟弹出一次"起来活动"，可调整或关闭；锁屏时间不计入。
- 面板展示最近 7 天专注时长柱状图。
- 关闭面板后，托盘和提醒继续工作。
- 记录每日专注时长和完成轮数，最多保留 90 个日期。
- 宠物名字可修改，面板标题、托盘提示和提醒窗口同步更新。
- 新配置默认浅色，支持深色和系统主题。

数据保存在本机，不需要账户或云服务。键鼠采样仅用于动画，不保存按键序列。检测到锁屏或休眠会暂停计时，重启后也恢复为暂停状态。

## 下载

- [Windows x64 EXE](https://github.com/lamar666666/lamar-pet/releases/latest/download/lamar-pet.exe)，双击即可运行。
- [macOS Apple Silicon（M 系列）](https://github.com/lamar666666/lamar-pet/releases/latest/download/lamar-pet-macos-arm64.zip)，解压得到 `LamarPet.app`。
- [macOS Intel](https://github.com/lamar666666/lamar-pet/releases/latest/download/lamar-pet-macos-x64.zip)，同上。

macOS 包未做公证签名，首次打开如提示"无法验证开发者"或"已损坏"，在解压目录执行 `xattr -dr com.apple.quarantine LamarPet.app` 后即可运行。

## 运行与构建

当前支持 Windows x64 与 macOS（12+，Apple Silicon / Intel）。源码使用 Rust 1.97.1 和 GPUI Kit。

Windows 构建需要 Rust、Visual Studio Build Tools 的 C++ 桌面开发组件及 Windows SDK；macOS 构建需要 Rust 和 Xcode Command Line Tools。

```powershell
cargo run --locked
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked --release
```

程序位于 `target/release/lamar-pet.exe`，角色资源已嵌入程序。关闭面板不会退出程序；退出请使用托盘菜单。

## 数据与恢复

数据目录为 `%LOCALAPPDATA%/cn.lamarlog.lamar-pet.sprite`。

`focus.json` 保存设置、会话和统计，`focus.json.bak` 保留上一份有效文件。状态切换、正常退出及运行期间每 15 秒保存。异常退出可能损失最近 15 秒。

遇到读取失败、损坏文件或不支持的数据版本时，程序停止覆盖并重试。修复前请先备份原文件；可用备份替换损坏的 `focus.json`。保存失败时保留现有文件并显示提示。

## 来源与许可

代码和项目角色资源采用 [Apache-2.0](LICENSE)。界面使用 [GPUI Kit](https://github.com/longbridge/gpui-kit)，桌面交互思路参考 [BongoCat](https://github.com/ayangweb/BongoCat)。第三方依赖保留各自许可证。

角色精灵图由 AI 辅助生成，程序采用普通 PNG 精灵帧，没有 Live2D SDK 或第三方角色模型。详见 [NOTICE](NOTICE.md)。