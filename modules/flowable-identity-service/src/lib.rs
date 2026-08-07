pub mod ldap;

use flowable_engine::engine::identity_service::{GroupQuery, TokenQuery, UserQuery};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::identity::entities::{Group, Privilege, Token, User, UserInfo, UserPicture};
use std::sync::Arc;

pub struct FlowableIdentityService {
    engine: Arc<ProcessEngine>,
}

impl FlowableIdentityService {
    pub fn new(engine: Arc<ProcessEngine>) -> Self {
        Self { engine }
    }

    fn identity_service(&self) -> Arc<flowable_engine::engine::identity_service::IdentityService> {
        self.engine.get_identity_service()
    }

    pub fn authenticate_password(&self, user_id: &str, password: &str) -> bool {
        self.identity_service().check_password(user_id, password)
    }

    pub fn save_user(&self, user: User) {
        self.identity_service().save_user(user)
    }

    pub fn find_user_by_id(&self, user_id: &str) -> Option<User> {
        self.identity_service().find_user_by_id(user_id)
    }

    pub fn delete_user(&self, user_id: &str) {
        self.identity_service().delete_user(user_id)
    }

    pub fn save_group(&self, group: Group) {
        self.identity_service().save_group(group)
    }

    pub fn find_group_by_id(&self, group_id: &str) -> Option<Group> {
        self.identity_service().find_group_by_id(group_id)
    }

    pub fn create_membership(&self, user_id: String, group_id: String) {
        self.identity_service().create_membership(user_id, group_id)
    }

    pub fn delete_membership(&self, user_id: &str, group_id: &str) {
        self.identity_service().delete_membership(user_id, group_id)
    }

    pub fn get_groups_by_user(&self, user_id: &str) -> Vec<Group> {
        self.identity_service().get_groups_by_user(user_id)
    }

    pub fn get_users_by_group(&self, group_id: &str) -> Vec<User> {
        self.identity_service().get_users_by_group(group_id)
    }

    pub fn membership_exists(&self, user_id: &str, group_id: &str) -> bool {
        self.identity_service().membership_exists(user_id, group_id)
    }

    pub fn create_user_query(&self) -> UserQuery {
        self.identity_service().create_user_query()
    }

    pub fn create_group_query(&self) -> GroupQuery {
        self.identity_service().create_group_query()
    }

    pub fn create_token_query(&self) -> TokenQuery {
        self.identity_service().create_token_query()
    }

    pub fn save_privilege(&self, privilege: Privilege) {
        self.identity_service().save_privilege(privilege)
    }

    pub fn find_privilege_by_id(&self, privilege_id: &str) -> Option<Privilege> {
        self.identity_service().find_privilege_by_id(privilege_id)
    }

    pub fn delete_privilege(&self, privilege_id: &str) {
        self.identity_service().delete_privilege(privilege_id)
    }

    pub fn add_user_privilege_mapping(&self, privilege_id: String, user_id: String) {
        self.identity_service()
            .add_user_privilege_mapping(privilege_id, user_id)
    }

    pub fn add_group_privilege_mapping(&self, privilege_id: String, group_id: String) {
        self.identity_service()
            .add_group_privilege_mapping(privilege_id, group_id)
    }

    pub fn delete_user_privilege_mapping(&self, privilege_id: &str, user_id: &str) {
        self.identity_service()
            .delete_user_privilege_mapping(privilege_id, user_id)
    }

    pub fn delete_group_privilege_mapping(&self, privilege_id: &str, group_id: &str) {
        self.identity_service()
            .delete_group_privilege_mapping(privilege_id, group_id)
    }

    pub fn get_privileges_for_user(&self, user_id: &str) -> Vec<Privilege> {
        self.identity_service().get_privileges_for_user(user_id)
    }

    pub fn get_privileges_for_group(&self, group_id: &str) -> Vec<Privilege> {
        self.identity_service().get_privileges_for_group(group_id)
    }

    pub fn save_token(&self, token: Token) {
        self.identity_service().save_token(token)
    }

    pub fn find_token_by_id(&self, token_id: &str) -> Option<Token> {
        self.identity_service().find_token_by_id(token_id)
    }

    pub fn delete_token(&self, token_id: &str) {
        self.identity_service().delete_token(token_id)
    }

    pub fn set_user_info(&self, user_id: String, key: String, value: String) -> UserInfo {
        self.identity_service().set_user_info(user_id, key, value)
    }

    pub fn get_user_info(&self, user_id: &str, key: &str) -> Option<UserInfo> {
        self.identity_service().get_user_info(user_id, key)
    }

    pub fn get_user_info_keys(&self, user_id: &str) -> Vec<String> {
        self.identity_service().get_user_info_keys(user_id)
    }

    pub fn delete_user_info(&self, user_id: &str, key: &str) -> bool {
        self.identity_service().delete_user_info(user_id, key)
    }

    pub fn set_user_picture(&self, user_id: String, mime_type: String, bytes: Vec<u8>) {
        self.identity_service()
            .set_user_picture(user_id, mime_type, bytes)
    }

    pub fn get_user_picture(&self, user_id: &str) -> Option<UserPicture> {
        self.identity_service().get_user_picture(user_id)
    }

    pub fn delete_user_picture(&self, user_id: &str) -> bool {
        self.identity_service().delete_user_picture(user_id)
    }
}
