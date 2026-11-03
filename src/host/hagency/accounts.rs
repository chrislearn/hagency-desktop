//! Closed account selections; native confirmation retains login authorization.
use super::{parse, Value};
use ::hagency::native_owner::{AccountInventoryQuery, PrepareAccount};
pub(super) const PREPARE: &str = "hagency.accounts.prepare";
pub(super) fn preparation(service: &str, args: Value) -> Result<Option<PrepareAccount>, String> {
    if service != PREPARE { return Ok(None); }
    let input: PrepareAccount = parse(args)?;
    input.validate().map_err(|_| "invalid_management_arguments".to_owned())?;
    Ok(Some(input))
}

pub(super) const LIST: &str = "hagency.accounts.list";
pub(super) const GET: &str = "hagency.accounts.get";

pub(super) fn query(service: &str, args: Value) -> Result<Option<AccountInventoryQuery>, String> {
    if !matches!(service, LIST | GET) { return Ok(None); }
    let input: AccountInventoryQuery = parse(args)?;
    input.validate().map_err(|_| "invalid_management_arguments".to_owned())?;
    if (service == GET) != input.id.is_some() {return Err("invalid_management_arguments".into());}
    Ok(Some(input))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::hagency::{permissions, ManagementPermission};
    use crate::miniapps::InstanceId;
    use serde_json::json;
    use std::time::{Duration, Instant};

    #[test]
    fn account_preparation_has_separate_closed_write_grant() {
        assert!(preparation(PREPARE,json!({"commandId":"original"})).unwrap().is_some());
        for extra in ["profile","owner","path","token","password","resourceId","runtime"] {
            let mut input=json!({"commandId":"original"});input[extra]=json!("forged");
            assert!(preparation(PREPARE,input).is_err());
        }
        assert!(preparation(PREPARE,json!({})).is_err());
        assert!(preparation(PREPARE,json!({"commandId":"../private"})).is_err());
        let authority=rinx_miniapp_core::SessionAuthority::default();
        let lease=authority.issue(InstanceId {app:octosense_app_contract::hagency::APP_ID.into(),account:"@owner:test".into(),room:None,generation:1},[PREPARE.to_owned()].into_iter().collect(),Default::default(),Instant::now()+Duration::from_secs(60));
        assert_eq!(permissions(&lease),vec![ManagementPermission::PrepareAccounts]);
        assert!(octosense_app_contract::hagency::SERVICES.contains(&PREPARE));
        assert!(octosense_app_contract::KNOWN_CAPABILITIES.contains(&PREPARE));
        assert!(octosense_app_contract::hagency::words(PREPARE).is_some());
    }

    #[test]
    fn account_inventory_is_a_closed_read_grant() {
        assert!(query(LIST,json!({})).unwrap().is_some());
        assert!(query(GET,json!({"id":"account_original","after":"resource_1"})).is_ok());
        assert!(query(LIST,json!({"id":"account_original"})).is_err());
        assert!(query(GET,json!({})).is_err());
        for field in ["owner","profile","path","url","token","credentialRef","retire","commandId"] {
            let mut input=json!({"id":"account_original"});input[field]=json!("forged");
            assert!(query(GET,input).is_err());
        }
        for id in ["", "../private", "two\nlines"] { assert!(query(GET,json!({"id":id})).is_err()); }
        let authority=rinx_miniapp_core::SessionAuthority::default();
        for service in [LIST, GET] {
            let lease=authority.issue(InstanceId {app:octosense_app_contract::hagency::APP_ID.into(),account:"@owner:test".into(),room:None,generation:1},[service.to_owned()].into_iter().collect(),Default::default(),Instant::now()+Duration::from_secs(60));
            assert_eq!(permissions(&lease),vec![ManagementPermission::ReadAccounts]);
            assert!(octosense_app_contract::hagency::SERVICES.contains(&service));
            assert!(octosense_app_contract::KNOWN_CAPABILITIES.contains(&service));
            assert!(octosense_app_contract::hagency::words(service).is_some());
        }
    }
}

pub(crate) const LOGIN: &str = "hagency.accounts.login";
pub(super) fn login_args(args: Value) -> Result<::hagency::native_owner::AccountLoginRequest,String> {
    let input: ::hagency::native_owner::AccountLoginRequest=parse(args)?;
    input.validate().map_err(|_|"invalid_management_arguments".to_owned())?;
    Ok(input)
}
/// The native sheet retains the current grant and exact original request.
pub(crate) struct AccountLoginFlow {
    host: super::Host,
    lease: super::Lease,
    input: ::hagency::native_owner::AccountLoginRequest,
    expires: std::time::Instant,
}
impl super::Host {
    pub fn account_login_flow(&self, lease: super::Lease, args: Value) -> Result<AccountLoginFlow,String> {
        let input=login_args(args)?;
        let account=self.check(&lease)?;lease.authorize(&account,LOGIN,None)?;
        Ok(AccountLoginFlow{host:self.clone(),lease,input,expires:std::time::Instant::now()+std::time::Duration::from_secs(600)})
    }
}
impl AccountLoginFlow {
    pub fn account(&self) -> &str {&self.lease.identity().account}
    pub fn namespace(&self) -> &str {&self.input.account_id}
    pub fn active(&self) -> bool {self.check().is_ok()}
    fn check(&self) -> Result<(),String> {
        let account=self.host.check(&self.lease)?;self.lease.authorize(&account,LOGIN,None)
    }
    pub fn cancelled_before_start(&self) -> ::hagency::native_owner::AccountLoginStatus {
        ::hagency::native_owner::AccountLoginStatus{command_id:self.input.command_id.clone(),account_id:self.input.account_id.clone(),state:"cancelled".into()}
    }
    async fn manager(&self) -> Result<std::sync::Arc<::hagency::daemon::bridge::ManagementClient>,String> {
        self.check()?;let manager=self.host.manager(&self.lease).await?;self.check()?;Ok(manager)
    }
    pub async fn start(&self) -> Result<::hagency::native_owner::AccountLoginStatus,String> {
        if std::time::Instant::now()>=self.expires {return Err("provider_confirmation_expired".into());}
        let result=self.manager().await?.login_account(self.input.clone()).await.map_err(|e|e.code);
        self.check().map_err(|_|"management_outcome_unknown".to_owned())?;result
    }
    pub async fn status(&self) -> Result<::hagency::native_owner::AccountLoginStatus,String> {
        let result=self.manager().await?.account_login_status(self.input.clone()).await.map_err(|e|e.code);self.check()?;result
    }
    pub async fn cancel(&self) -> Result<::hagency::native_owner::AccountLoginStatus,String> {
        let result=self.manager().await?.cancel_account_login(self.input.clone()).await.map_err(|e|e.code);self.check()?;result
    }
    pub async fn authorization(&self) -> Result<::hagency::daemon::bridge::AccountLoginAuthorization,String> {
        let result=self.manager().await?.account_login_authorization(self.input.clone()).await.map_err(|e|e.code);self.check()?;result
    }
}

#[cfg(test)]
mod login_tests {
 use super::*;
 use crate::host::hagency::{permissions,ManagementPermission};
 use crate::miniapps::InstanceId;
 use serde_json::json;
 use std::time::{Duration,Instant};
 #[test]
 fn account_login_grant_and_request_exclude_script_credentials_and_execution() {
  let valid=json!({"commandId":"original","accountId":"account_selected","expectedRevision":"a".repeat(64)});
  assert!(login_args(valid.clone()).is_ok());
  for field in ["url","authUrl","token","code","path","provider","owner","profile","executable","approved","verdict"] {let mut input=valid.clone();input[field]=json!("forged");assert!(login_args(input).is_err());}
  let authority=rinx_miniapp_core::SessionAuthority::default();
  let lease=authority.issue(InstanceId{app:octosense_app_contract::hagency::APP_ID.into(),account:"@owner:test".into(),room:None,generation:1},[LOGIN.to_owned()].into_iter().collect(),Default::default(),Instant::now()+Duration::from_secs(60));
  assert_eq!(permissions(&lease),vec![ManagementPermission::LoginAccounts]);
  assert!(octosense_app_contract::hagency::SERVICES.contains(&LOGIN));assert!(octosense_app_contract::KNOWN_CAPABILITIES.contains(&LOGIN));
 }
}

pub(super) const MODELS:&str="hagency.accounts.models";
pub(super) fn models(service:&str,args:Value)->Result<Option<::hagency::native_owner::AccountModelQuery>,String>{
    if service!=MODELS {return Ok(None);}
    let query: ::hagency::native_owner::AccountModelQuery=parse(args)?;
    query.validate().map_err(|_|"invalid_management_arguments".to_owned())?;
    Ok(Some(query))
}
#[cfg(test)]
mod model_tests {
    use super::*;
    use crate::host::hagency::{permissions,ManagementPermission};
    use crate::miniapps::InstanceId;
    use serde_json::json;
    use std::time::{Duration,Instant};
    #[test]
    fn account_models_grant_has_no_login_enrollment_or_path_authority(){
        let valid=json!({"accountId":"account_selected","expectedRevision":"a".repeat(64)});
        assert!(models(MODELS,valid.clone()).unwrap().is_some());
        for field in ["path","profile","owner","commandId","runtime","executable","credentialRef","url","token"]{
            let mut value=valid.clone();value[field]=json!("forged");assert!(models(MODELS,value).is_err());
        }
        let authority=rinx_miniapp_core::SessionAuthority::default();
        let lease=authority.issue(InstanceId{app:octosense_app_contract::hagency::APP_ID.into(),account:"@owner:test".into(),room:None,generation:1},[MODELS.to_owned()].into_iter().collect(),Default::default(),Instant::now()+Duration::from_secs(60));
        assert_eq!(permissions(&lease),vec![ManagementPermission::ReadAccountModels]);
        assert!(octosense_app_contract::hagency::SERVICES.contains(&MODELS));assert!(octosense_app_contract::KNOWN_CAPABILITIES.contains(&MODELS));assert!(octosense_app_contract::hagency::words(MODELS).is_some());
    }
}
