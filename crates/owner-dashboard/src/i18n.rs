//! English and Simplified Chinese strings. Every key has both translations:
//! the `match` in [`text`] is exhaustive, so a missing translation fails to
//! compile rather than falling back silently.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Locale {
    #[default]
    En,
    ZhHans,
}

impl Locale {
    /// Maps a BCP 47 tag or POSIX locale (`zh-CN`, `zh_Hans`, `en_US.UTF-8`).
    /// Any Chinese tag selects Simplified Chinese; everything else English.
    pub fn from_tag(tag: &str) -> Self {
        let lower = tag.trim().to_ascii_lowercase();
        if lower.starts_with("zh") {
            Self::ZhHans
        } else {
            Self::En
        }
    }
}

macro_rules! messages {
    ($($key:ident => $en:literal, $zh:literal;)+) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Msg { $($key),+ }
        impl Msg {
            pub const ALL: &'static [Msg] = &[$(Msg::$key),+];
        }
        pub fn text(locale: Locale, msg: Msg) -> &'static str {
            match (locale, msg) {
                $((Locale::En, Msg::$key) => $en, (Locale::ZhHans, Msg::$key) => $zh,)+
            }
        }
    };
}

messages! {
    Title => "Hosting", "托管";
    Refresh => "Refresh", "刷新";
    StatusWorking => "Working…", "正在处理…";
    StatusUnconfirmed => "Not confirmed", "尚未确认";
    StatusAttention => "Needs attention", "需要处理";
    StatusDone => "Done", "已完成";
    StatusUpdate => "Update", "状态更新";
    StatusConnecting => "Connecting…", "正在连接…";
    StatusInventory => "Check resources", "资源列表需检查";
    StatusOffline => "Service offline", "服务未运行";
    StatusDisconnected => "Disconnected", "连接已断开";
    StatusOtherOwner => "Another owner", "其他所有者";
    Refreshing => "Refreshing…", "正在刷新…";
    OpenInHagency => "Dock back", "停靠回 Hagency";
    OpenStandalone => "Open separately", "独立打开";
    TabOverview => "Overview", "概览";
    TabResources => "Resources", "资源";
    TabUsage => "Usage", "用量";
    TabActivity => "Activity", "活动";
    HideFilters => "Hide filters", "收起筛选";
    UsageFilters => "Filters & dates", "筛选与日期";
    CustomDates => "Custom dates", "自选日期";
    StartDate => "Start date · UTC", "开始日期 · UTC";
    EndDate => "End date · UTC", "结束日期 · UTC";
    ApplyDates => "Apply dates", "应用日期";
    DateRangeError => "Use YYYY-MM-DD. The end must follow the start, within 366 days.", "请使用 YYYY-MM-DD 格式，结束日期不得早于开始日期，范围最多 366 天。";
    FilterResource => "Resource", "资源";
    CodingAccount => "Coding account", "编码账号";
    Filtered => "Filtered", "已筛选";
    InventoryUnavailable => "Resource list unavailable", "资源列表暂不可用";
    InventoryRetry => "Refresh to try loading your resources again.", "请刷新以重新加载资源。";
    InventoryStale => "Refresh failed. Previously loaded resources are shown below.", "刷新失败，下方显示上次加载的资源。";
    InventoryPartial => "Resource list is incomplete. Usage totals may include resources not listed here.", "资源列表尚不完整。用量总计可能包含未列出的资源。";
    FilterModel => "Model", "模型";
    FilterRequester => "Requester", "请求者";
    AllResources => "All resources", "所有资源";
    AllModels => "All models", "所有模型";
    AllRequesters => "All requesters", "所有请求者";
    ClearFilters => "Clear filters", "清除筛选";

    ConnectingTitle => "Connecting to hosting service…", "正在连接托管服务…";
    ConnectingBody => "Your Agents keep running in the background while this window opens.", "窗口打开期间，你的 Agent 会继续在后台运行。";
    DaemonAbsentTitle => "Hosting service isn't running", "托管服务未运行";
    UnsupportedPlatformTitle => "Background hosting is available on macOS", "后台托管目前支持 macOS";
    UnsupportedPlatformBody => "This release does not include a Windows resource daemon. You can still chat with Agents hosted on another execution device.", "此版本尚未包含 Windows 资源 daemon。你仍可与其他执行设备托管的 Agent 聊天。";
    DaemonAbsentBody => "Hagency hosts your rented Agents with a background service that keeps working when Hagency is closed. Set it up or start it to continue.", "Hagency 通过后台服务托管你出租的 Agent，即使关闭 Hagency 也会继续工作。请设置或启动该服务。";
    IncompatibleTitle => "Update required", "需要更新";
    AuthExpiredTitle => "Sign-in needed", "需要重新登录";
    AuthExpiredBody => "The hosting service's owner sign-in has expired. Sign in again to keep accepting work.", "托管服务的所有者登录已过期。请重新登录以继续接收任务。";
    OwnerMismatchTitle => "Hosting belongs to another account", "托管服务属于其他账号";
    OwnerMismatchBody => "The hosting service on this Mac serves a different owner. Its data isn't shown here. Switch to that account, or set up hosting for this one.", "此 Mac 上的托管服务属于另一位所有者，因此不在此显示其数据。请切换到该账号，或为当前账号设置托管。";
    FailedTitle => "Can't reach the hosting service", "无法连接托管服务";
    ShowingLastKnown => "Showing the last known status.", "当前显示的是最后一次获取的状态。";
    SetUpHosting => "Set up hosting", "设置托管";
    SignInAgain => "Sign in again", "重新登录";
    TryAgain => "Try again", "重试";

    StateUnconfiguredTitle => "Hosting isn't set up", "尚未设置托管";
    StateUnconfiguredBody => "Set up owner hosting to serve your rented Agents while Hagency is closed.", "设置所有者托管后，即使关闭 Hagency 也能为出租的 Agent 提供服务。";
    StateStartingTitle => "Starting hosting", "正在启动托管";
    StateStartingBody => "Connecting workers. New work is accepted once hosting is on.", "正在连接工作进程。托管开启后才会接收新任务。";
    StateRunningTitle => "Hosting is on", "托管已开启";
    StateRunningBody => "Accepting new work for your rented Agents.", "正在为出租的 Agent 接收新任务。";
    StatePausedTitle => "Hosting is paused", "托管已暂停";
    StatePausedBody => "No new work is accepted. Resume to accept requests again.", "不再接收新任务。恢复后会重新接收请求。";
    StateDrainingTitle => "Pausing", "正在暂停";
    StateDrainingBody => "No new work is accepted. Running tasks are finishing.", "不再接收新任务，正在运行的任务即将完成。";
    StateStoppedTitle => "Hosting is stopped", "托管已停止";
    StateStoppedBody => "Your resources and settings are saved. Resume hosting when you are ready.", "资源和设置已保留，您可以随时恢复托管。";
    StateOfflineTitle => "Hosting is offline", "托管离线";
    StateOfflineBody => "The Hagency server can't be reached. Work resumes when the connection returns.", "无法连接 Hagency 服务器。连接恢复后会继续工作。";
    StateFailedTitle => "Hosting stopped with an error", "托管因错误停止";
    StateFailedBody => "Review the details, then check setup.", "请查看详细信息，然后检查设置。";

    ActiveRuns => "Running", "运行中";
    PendingApprovals => "Awaiting approval", "待审批";
    PauseHosting => "Pause hosting", "暂停托管";
    ResumeHosting => "Resume hosting", "恢复托管";
    StopHosting => "Stop hosting", "停止托管";
    Stop => "Stop", "停止";
    ReviewApprovals => "Review approvals", "查看审批";
    ReviewInHagency => "Review in Hagency", "在 Hagency 中查看";
    StopConfirmTitle => "Stop hosting?", "停止托管？";
    StopConfirmBody => "New work stops. Running tasks may be interrupted.", "停止接收新任务，运行中的任务可能被中断。";
    Cancel => "Cancel", "取消";
    CheckStatus => "Check status", "查询状态";
    Retry => "Retry", "重试";
    Dismiss => "Dismiss", "关闭";

    PlansTitle => "Provider plans", "服务商套餐";
    PlansEmpty => "No provider accounts reported yet. Plan limits appear after the hosting service observes an account.", "尚未报告服务商账号。托管服务观测到账号后会显示套餐限额。";
    NotReported => "Not reported", "未报告";
    ResetNotReported => "Reset time not reported", "未报告重置时间";
    NeverObserved => "Not observed yet", "尚未观测";
    AccountUnverifiedNote => "Account identity isn't verified, so it isn't combined with other accounts.", "账号身份未经验证，因此不会与其他账号合并。";
    UnassignedAccount => "Account not verified", "账号未验证";
    UnassignedAccountNote => "These resources aren't linked to a verified provider account. Their usage isn't counted against any plan above.", "这些资源未关联到已验证的服务商账号，其用量不计入上方任何套餐。";
    ResourcesOnAccount => "Resources on this account", "使用此账号的资源";

    AccountFresh => "Up to date", "最新";
    AccountStale => "Out of date", "已过时";
    AccountPartial => "Partial data", "部分数据";
    AccountLimited => "Limit reached", "已达限额";
    AccountUnsupported => "Not reported by provider", "服务商不提供";
    AccountUnknown => "Not observed yet", "尚未观测";
    AccountUnverified => "Identity not verified", "身份未验证";
    StatusUnrecognized => "Unrecognized status", "无法识别的状态";

    UsageTitle => "Measured usage", "实测用量";
    UsageTotal => "Total tokens", "总 Token";
    TokensInput => "Input", "输入";
    TokensOutput => "Output", "输出";
    TokensCacheRead => "Cache read", "缓存读取";
    TokensCacheWrite => "Cache write", "缓存写入";
    CacheHitRatio => "Cache hit ratio", "缓存命中率";
    Unavailable => "Unavailable", "不可用";
    Unknown => "Unknown", "未知";
    UsageComplete => "All measurements complete", "所有测量均完整";
    UsagePartial => "Partial: some measurements are missing, so totals are a lower bound.", "部分数据：缺少部分测量，总量为下限。";
    UsageNoData => "No measured usage reported.", "尚无实测用量。";
    EstimatedCost => "API-equivalent estimate", "按 API 价格估算";
    EstimatedCostNote => "Not a subscription charge.", "并非订阅费用。";
    NotEstimated => "Not estimated", "未估算";
    PeriodDay => "Today", "今天";
    PeriodWeek => "This week", "本周";
    PeriodMonth => "This month", "本月";
    PeriodUnreported => "Period not reported", "未报告统计周期";
    ByResource => "By resource", "按资源";
    ByAccount => "By account", "按账号";
    AllAccounts => "All accounts", "全部账号";
    UsageBoundary => "Calendar periods are for reporting only; providers apply their own limit windows.", "日历周期仅用于统计；服务商按各自的限额窗口计算。";

    ResourcesTitle => "Resources", "资源";
    ResourcesEmpty => "No hosted resources yet. Resources appear after you publish an Agent pool.", "暂无托管资源。发布 Agent 资源池后会显示在这里。";
    Agents => "Agents", "Agent 数量";
    Budget => "Budget", "预算";
    NoBudget => "Budget not reported", "未提供预算信息";
    TokensUsed => "Tokens used", "已用 Token";
    ResourceMeasured => "Measured", "已测量";
    ResourcePartial => "Partially measured", "部分测量";
    ResourceUnmeasured => "Not measured", "未测量";
    ResourceIdle => "Idle", "空闲";
    Details => "Details", "详情";
    Back => "Back", "返回";
    TechnicalDetails => "Technical details", "技术信息";
    ResourceId => "Resource ID", "资源 ID";
    AccountId => "Account ID", "账号 ID";
    Provider => "Provider", "服务商";
    Model => "Model", "模型";
    ResourceGone => "This resource is no longer reported by the hosting service.", "托管服务已不再报告此资源。";

    ActivityTitle => "Activity", "活动";
    ActivityEmpty => "No recent activity.", "暂无近期活动。";
    ActivityRunning => "Running", "运行中";
    ActivityPending => "Pending", "等待中";
    ActivitySucceeded => "Done", "已完成";
    ActivityFailed => "Failed", "失败";
    ActivityUncertain => "Outcome unknown", "结果未知";
    ActivityInfo => "Info", "信息";
    ActivityWarning => "Warning", "警告";

    AttentionTitle => "Needs attention", "需要处理";
    AttentionNone => "Nothing needs your attention.", "暂无需要处理的事项。";
    AttentionPartialUsage => "Some usage measurements are incomplete.", "部分用量测量不完整。";
    AttentionUnassigned => "Some resources aren't linked to a verified account.", "部分资源未关联到已验证的账号。";

    OpeningHagency => "Opening Hagency…", "正在打开 Hagency…";
    WindowTitle => "Hagency Hosting", "Hagency 托管";
    JustNow => "just now", "刚刚";
    Today => "Today", "今天";
    Tomorrow => "Tomorrow", "明天";
    SignedInAs => "Owner", "所有者";
    NoOwner => "No owner signed in", "未登录所有者";

    ReasonRevisionConflict => "Hosting changed. Review and retry.", "托管状态已变化，请查看后重试。";
    ReasonOwnerScope => "This controller isn't authorized for the signed-in owner.", "此控制器未获得当前所有者的授权。";
    ReasonStopped => "The hosting service is stopped.", "托管服务已停止。";
    ReasonBusy => "The hosting service is busy. Try again shortly.", "托管服务繁忙，请稍后重试。";
    ReasonNotSupported => "This hosting service doesn't support that action.", "此托管服务不支持该操作。";
    ReasonShuttingDown => "The hosting service is shutting down.", "托管服务正在关闭。";
    ReasonVersion => "The hosting service uses a different version. Update Hagency.", "托管服务版本不一致，请更新 Hagency。";
    ReasonUnknown => "The hosting service rejected the request.", "托管服务拒绝了该请求。";
    ReasonNotConnected => "Not connected to the hosting service.", "未连接到托管服务。";
}

pub fn tr(locale: Locale, msg: Msg) -> &'static str {
    text(locale, msg)
}

/// Replaces `{name}` placeholders. Unknown placeholders are left visible so a
/// translation mistake is noticed rather than silently dropping data.
pub fn fill(template: &str, args: &[(&str, &str)]) -> String {
    let mut out = template.to_owned();
    for (name, value) in args {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

/// Parameterized phrases. Each returns a complete sentence in the locale.
pub mod phrase {
    use super::{Locale, fill};

    pub fn updated(locale: Locale, age: &str) -> String {
        match locale {
            Locale::En => fill("Updated {age}", &[("age", age)]),
            Locale::ZhHans => fill("更新于{age}", &[("age", age)]),
        }
    }
    pub fn observed(locale: Locale, age: &str) -> String {
        match locale {
            Locale::En => fill("Observed {age}", &[("age", age)]),
            Locale::ZhHans => fill("观测于{age}", &[("age", age)]),
        }
    }
    pub fn used_percent(locale: Locale, percent: &str) -> String {
        match locale {
            Locale::En => fill("{p} used", &[("p", percent)]),
            Locale::ZhHans => fill("已用 {p}", &[("p", percent)]),
        }
    }
    pub fn resets(locale: Locale, remaining: &str, at: &str) -> String {
        match locale {
            Locale::En => fill("Resets in {r} · {at}", &[("r", remaining), ("at", at)]),
            Locale::ZhHans => fill("{r}后重置 · {at}", &[("r", remaining), ("at", at)]),
        }
    }
    pub fn reset_due(locale: Locale, at: &str) -> String {
        match locale {
            Locale::En => fill("Reset due {at} · awaiting new observation", &[("at", at)]),
            Locale::ZhHans => fill("应于 {at} 重置 · 等待新的观测", &[("at", at)]),
        }
    }
    pub fn incompatible(locale: Locale, ours: u32, theirs: Option<u32>) -> String {
        let theirs = theirs.map(|v| v.to_string());
        match (locale, theirs) {
            (Locale::En, Some(v)) => format!("This dashboard uses API version {ours}; the hosting service reports version {v}. Update Hagency so both match."),
            (Locale::En, None) => format!("This dashboard uses API version {ours}; the hosting service didn't report a compatible version. Update Hagency so both match."),
            (Locale::ZhHans, Some(v)) => format!("此面板使用 API 版本 {ours}，托管服务报告的版本为 {v}。请更新 Hagency 使两者一致。"),
            (Locale::ZhHans, None) => format!("此面板使用 API 版本 {ours}，托管服务未报告兼容版本。请更新 Hagency 使两者一致。"),
        }
    }
    pub fn running_and_pending(locale: Locale, running: u64, pending: u64) -> String {
        match locale {
            Locale::En => format!("{running} running · {pending} awaiting approval"),
            Locale::ZhHans => format!("{running} 个运行中 · {pending} 个待审批"),
        }
    }
    pub fn draining(locale: Locale, running: u64) -> String {
        match locale {
            Locale::En if running == 1 => "1 running task is finishing; no new work is accepted.".into(),
            Locale::En => format!("{running} running tasks are finishing; no new work is accepted."),
            Locale::ZhHans => format!("{running} 个正在运行的任务即将完成；不再接收新任务。"),
        }
    }
    pub fn submitting(locale: Locale, action: &str) -> String {
        match locale {
            Locale::En => format!("{action}: sending…"),
            Locale::ZhHans => format!("{action}：正在发送…"),
        }
    }
    pub fn awaiting(locale: Locale, action: &str) -> String {
        match locale {
            Locale::En => format!("{action}: accepted, waiting for the hosting service to confirm…"),
            Locale::ZhHans => format!("{action}：已接受，等待托管服务确认…"),
        }
    }
    pub fn awaiting_drain(locale: Locale, action: &str, running: u64) -> String {
        match locale {
            Locale::En => format!("{action}: accepted. Waiting for {running} running task(s) to finish."),
            Locale::ZhHans => format!("{action}：已接受，等待 {running} 个运行中的任务完成。"),
        }
    }
    pub fn confirmed(locale: Locale, action: &str, at: &str) -> String {
        match locale {
            Locale::En => format!("{action}: confirmed {at}."),
            Locale::ZhHans => format!("{action}：已于 {at} 确认。"),
        }
    }
    pub fn unchanged(locale: Locale, action: &str) -> String {
        match locale {
            Locale::En => format!("{action}: no change was needed."),
            Locale::ZhHans => format!("{action}：无需更改。"),
        }
    }
    pub fn rejected(locale: Locale, action: &str, reason: &str, code: &str) -> String {
        match locale {
            Locale::En => format!("{action} failed: {reason} ({code})"),
            Locale::ZhHans => format!("{action}失败：{reason}（{code}）"),
        }
    }
    pub fn uncertain(locale: Locale, action: &str) -> String {
        match locale {
            Locale::En => format!("{action}: outcome unknown. Check status without sending again."),
            Locale::ZhHans => format!("{action}：结果尚未确认。查询状态不会重复发送。"),
        }
    }
    pub fn checking(locale: Locale, action: &str) -> String {
        match locale {
            Locale::En => format!("{action}: checking the outcome…"),
            Locale::ZhHans => format!("{action}：正在查询结果…"),
        }
    }
    pub fn open_failed(locale: Locale, action: &str, reason: &str) -> String {
        match locale {
            Locale::En => format!("{action} failed: {reason}"),
            Locale::ZhHans => format!("{action}失败：{reason}"),
        }
    }
    pub fn approvals_waiting(locale: Locale, n: u64) -> String {
        match locale {
            Locale::En if n == 1 => "1 request is awaiting your approval.".into(),
            Locale::En => format!("{n} requests are awaiting your approval."),
            Locale::ZhHans => format!("{n} 个请求正在等待你的审批。"),
        }
    }
    pub fn account_limited(locale: Locale, label: &str) -> String {
        match locale {
            Locale::En => format!("{label} reached a provider limit."),
            Locale::ZhHans => format!("{label} 已达到服务商限额。"),
        }
    }
    pub fn account_near_limit(locale: Locale, label: &str, percent: &str) -> String {
        match locale {
            Locale::En => format!("{label} has used {percent} of a plan window."),
            Locale::ZhHans => format!("{label} 的某个套餐窗口已用 {percent}。"),
        }
    }
    pub fn failed_activities(locale: Locale, n: usize) -> String {
        match locale {
            Locale::En if n == 1 => "1 activity failed or has an unknown outcome.".into(),
            Locale::En => format!("{n} activities failed or have an unknown outcome."),
            Locale::ZhHans => format!("{n} 项活动失败或结果未知。"),
        }
    }
    pub fn resource_line(locale: Locale, provider: &str, model: &str, agents: Option<u64>) -> String {
        match (locale, agents) {
            (Locale::En, Some(1)) => format!("{provider} · {model} · 1 Agent"),
            (Locale::En, Some(n)) => format!("{provider} · {model} · {n} Agents"),
            (Locale::ZhHans, Some(n)) => format!("{provider} · {model} · {n} 个 Agent"),
            (Locale::En, None) => format!("{provider} · {model} · Agent count unavailable"),
            (Locale::ZhHans, None) => format!("{provider} · {model} · Agent 数量暂不可用"),
        }
    }
    pub fn budget_of(locale: Locale, _used: &str, budget: &str) -> String {
        match locale {
            Locale::En => format!("Owner limit: {budget} tokens"),
            Locale::ZhHans => format!("主人设置的上限：{budget} Token"),
        }
    }
    pub fn approx_usd(_locale: Locale, value: &str) -> String {
        format!("≈ {value}")
    }
    pub fn window_label(locale: Locale, label: &str, length: Option<&str>) -> String {
        match (locale, length) {
            (_, None) => label.to_owned(),
            (Locale::En, Some(l)) => format!("{label} · {l}"),
            (Locale::ZhHans, Some(l)) => format!("{label} · {l}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_message_is_translated_and_distinct_per_locale() {
        for &msg in Msg::ALL {
            let en = text(Locale::En, msg);
            let zh = text(Locale::ZhHans, msg);
            assert!(!en.trim().is_empty(), "{msg:?} has empty English");
            assert!(!zh.trim().is_empty(), "{msg:?} has empty Chinese");
            assert!(
                zh.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)) || zh == en,
                "{msg:?} Chinese text has no CJK characters: {zh}"
            );
        }
    }

    #[test]
    fn locale_tags_map_to_supported_locales() {
        assert_eq!(Locale::from_tag("zh-CN"), Locale::ZhHans);
        assert_eq!(Locale::from_tag("zh_Hans_CN.UTF-8"), Locale::ZhHans);
        assert_eq!(Locale::from_tag("ZH-hant"), Locale::ZhHans);
        assert_eq!(Locale::from_tag("en_US.UTF-8"), Locale::En);
        assert_eq!(Locale::from_tag(""), Locale::En);
    }

    #[test]
    fn fill_keeps_unknown_placeholders_visible() {
        assert_eq!(fill("{a} and {b}", &[("a", "1")]), "1 and {b}");
    }
}
