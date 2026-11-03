//! Localize the daemon's own event vocabulary. User-authored resource/room
//! names and unknown future events are preserved; they are not UI messages.
use crate::{Locale, dto::Activity};

pub fn localized(event: &Activity, locale: Locale) -> (String, String) {
    let owned = event.id.strip_prefix("daemon-").and_then(|id| id.split_once('-'))
        .is_some_and(|(time, seq)| time.parse::<u64>().is_ok() && seq.parse::<u64>().is_ok());
    if locale != Locale::ZhHans || !owned { return (event.title.clone(), event.detail.clone()); }
    let title = match event.title.as_str() {
        "Owner signed out" => "资源主人已退出登录",
        "Refreshing provider reports" => "正在刷新编码账号报告",
        "Provider reports refreshed" => "编码账号报告已刷新",
        "Provider reports unavailable" => "暂时无法获取编码账号报告",
        "Hosting could not start" => "托管启动失败",
        "Usage history unavailable" => "暂时无法读取历史用量",
        "Hosting started" => "托管已启动",
        "Hosting stopped" => "托管已停止",
        "Hosting stop needs recovery" => "停止托管尚未完成",
        "Owner authorization expired" => "资源主人授权已过期",
        _ => return (event.title.clone(), event.detail.clone()),
    };
    let detail = match event.detail.as_str() {
        "Daemon enrollment withdrawn; owner credentials revoked." => "后台服务连接已撤销，资源主人凭据已失效。".into(),
        "Reading current reports from the original coding accounts." => "正在读取已连接编码账号的最新报告。".into(),
        "No current supported provider report was available. Existing reports keep their observation times." => "暂时没有可读取的最新报告。已有报告仍显示原来的采集时间。".into(),
        "Pool workers started." => "资源池执行服务已启动。".into(),
        "Owner authorized." => "资源主人已授权。".into(),
        "Workers drained and the owner root was released." => "执行服务已停止，工作目录已释放。".into(),
        "telemetry_storage_unavailable" => "无法读取已保存用量；当前数据可能不完整。（telemetry_storage_unavailable）".into(),
        "matrix_authorization_required" => "请重新连接资源主人账号。（matrix_authorization_required）".into(),
        value => reports(value).unwrap_or_else(|| {
            if matches!(event.title.as_str(), "Hosting could not start" | "Hosting stop needs recovery") {
                format!("{}（{}）", crate::model::rejection_reason(value, locale), value)
            } else { value.to_owned() }
        }),
    };
    (title.into(), detail)
}

fn reports(value: &str) -> Option<String> {
    let (updated, rest) = value.split_once(" resource reports updated. ")?;
    let unavailable = rest.strip_suffix(" unavailable; previous reports keep their observation times.")?;
    let updated = updated.parse::<u64>().ok()?;
    let unavailable = unavailable.parse::<u64>().ok()?;
    Some(format!("已更新 {updated} 份资源报告，{unavailable} 份暂不可用。已有报告仍显示原来的采集时间。"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(id: &str, title: &str, detail: &str) -> Activity {
        Activity { id: id.into(), title: title.into(), detail: detail.into(), at_ms: 1, state: "succeeded".into() }
    }
    #[test]
    fn localizes_owned_messages_and_exact_report_counts_only() {
        let row = event("daemon-123-1", "Provider reports refreshed", "3 resource reports updated. 2 unavailable; previous reports keep their observation times.");
        let (title, detail) = localized(&row, Locale::ZhHans);
        assert_eq!(title, "编码账号报告已刷新");
        assert!(detail.starts_with("已更新 3 份资源报告，2 份暂不可用。"));
        assert_eq!(localized(&row, Locale::En), (row.title, row.detail));
    }
    #[test]
    fn user_content_and_future_events_are_never_reinterpreted() {
        for row in [event("user-123", "Hosting started", "Owner authorized."),
                    event("daemon-123-1", "My custom Agent title", "Project Atlas"),
                    event("daemon-forged-1", "Hosting started", "Owner authorized.")] {
            assert_eq!(localized(&row, Locale::ZhHans), (row.title, row.detail));
        }
    }
}
