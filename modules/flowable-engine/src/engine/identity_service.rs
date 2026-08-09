use crate::engine::query::{Direction, Query, QueryState};
use crate::identity::entities::*;
use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;
use crate::interceptor::command_executor::{CommandExecutor, DefaultCommandExecutor};
use crate::persistence::db_session::DbSession;
use std::sync::Arc;

pub struct IdentityService {
    command_executor: Arc<DefaultCommandExecutor>,
}

impl IdentityService {
    pub fn new(command_executor: Arc<DefaultCommandExecutor>) -> Self {
        Self { command_executor }
    }

    fn get_store(&self) -> crate::persistence::runtime_store::RuntimeStore {
        self.command_executor.runtime_store().clone()
    }

    fn create_session(&self) -> DbSession {
        self.get_store().create_session().unwrap()
    }

    pub fn check_password(&self, user_id: &str, password: &str) -> bool {
        let mut session = self.create_session();
        let result = self.check_password_in_session(user_id, password, &mut session);
        let _ = session.rollback();
        result
    }

    pub fn check_password_in_session(
        &self,
        user_id: &str,
        password: &str,
        session: &mut DbSession,
    ) -> bool {
        if let Some(user) = self.get_store().find_user(user_id, session) {
            match user.password.as_deref() {
                Some(stored) => crate::identity::password::verify_password(password, stored),
                None => false,
            }
        } else {
            false
        }
    }

    pub fn save_user(&self, user: User) {
        let mut session = self.create_session();
        self.save_user_in_session(user, &mut session);
        session.flush_and_commit().unwrap();
    }

    /// Persist a user. Plaintext passwords are argon2id-hashed before the
    /// entity is written to the store (security deviation from Java plaintext
    /// storage); values that are already *well-formed* hashes are stored
    /// unchanged so update flows that re-save a loaded user never double-hash.
    ///
    /// The guard parses rather than prefix-matches: a chosen password that
    /// merely starts with `$argon2id$` must still be hashed, or it would land in
    /// the database as plaintext and be unverifiable afterwards.
    pub fn save_user_in_session(&self, mut user: User, session: &mut DbSession) {
        // The password is only moved out when it actually needs hashing.
        // `take()`-ing first would drop an already-hashed value on the floor
        // whenever the `&&` short-circuits, wiping the password of every user
        // re-saved from a loaded entity.
        let needs_hashing = user
            .password
            .as_deref()
            .is_some_and(|value| !crate::identity::password::is_valid_hash(value));
        if needs_hashing {
            let plain = user
                .password
                .take()
                .expect("presence was just established");
            user.password = Some(crate::identity::password::hash_password(&plain));
        }
        self.get_store().insert_user(user, session);
    }

    pub fn find_user_by_id(&self, user_id: &str) -> Option<User> {
        let mut session = self.create_session();
        let result = self.find_user_by_id_in_session(user_id, &mut session);
        let _ = session.rollback();
        result
    }

    pub fn find_user_by_id_in_session(
        &self,
        user_id: &str,
        session: &mut DbSession,
    ) -> Option<User> {
        self.get_store().find_user(user_id, session)
    }

    pub fn delete_user(&self, user_id: &str) {
        let mut session = self.create_session();
        self.delete_user_in_session(user_id, &mut session);
        session.flush_and_commit().unwrap();
    }

    pub fn delete_user_in_session(&self, user_id: &str, session: &mut DbSession) {
        self.get_store().delete_user(user_id, session);
    }

    pub fn set_user_info(&self, user_id: String, key: String, value: String) -> UserInfo {
        let mut session = self.create_session();
        let result = self.set_user_info_in_session(user_id, key, value, &mut session);
        session.flush_and_commit().unwrap();
        result
    }

    pub fn set_user_info_in_session(
        &self,
        user_id: String,
        key: String,
        value: String,
        session: &mut DbSession,
    ) -> UserInfo {
        let now = chrono::Utc::now().timestamp_millis();
        let existing = self.get_store().find_user_info(&user_id, &key, session);
        let info = UserInfo {
            user_id,
            key,
            value,
            created_at: existing.and_then(|e| e.created_at).or(Some(now)),
            updated_at: Some(now),
        };
        self.get_store().insert_user_info(info.clone(), session);
        info
    }

    pub fn get_user_info(&self, user_id: &str, key: &str) -> Option<UserInfo> {
        let mut session = self.create_session();
        let result = self.get_user_info_in_session(user_id, key, &mut session);
        let _ = session.rollback();
        result
    }

    pub fn get_user_info_in_session(
        &self,
        user_id: &str,
        key: &str,
        session: &mut DbSession,
    ) -> Option<UserInfo> {
        self.get_store().find_user_info(user_id, key, session)
    }

    pub fn get_user_info_keys(&self, user_id: &str) -> Vec<String> {
        let mut session = self.create_session();
        let result = self.get_user_info_keys_in_session(user_id, &mut session);
        let _ = session.rollback();
        result
    }

    pub fn get_user_info_keys_in_session(
        &self,
        user_id: &str,
        session: &mut DbSession,
    ) -> Vec<String> {
        self.get_store()
            .list_user_info(user_id, session)
            .into_iter()
            .map(|info| info.key)
            .collect()
    }

    pub fn delete_user_info(&self, user_id: &str, key: &str) -> bool {
        let mut session = self.create_session();
        let result = self.delete_user_info_in_session(user_id, key, &mut session);
        session.flush_and_commit().unwrap();
        result
    }

    pub fn delete_user_info_in_session(
        &self,
        user_id: &str,
        key: &str,
        session: &mut DbSession,
    ) -> bool {
        self.get_store().delete_user_info(user_id, key, session)
    }

    pub fn set_user_picture(&self, user_id: String, mime_type: String, bytes: Vec<u8>) {
        let mut session = self.create_session();
        self.set_user_picture_in_session(user_id, mime_type, bytes, &mut session);
        session.flush_and_commit().unwrap();
    }

    pub fn set_user_picture_in_session(
        &self,
        user_id: String,
        mime_type: String,
        bytes: Vec<u8>,
        session: &mut DbSession,
    ) {
        let now = chrono::Utc::now().timestamp_millis();
        self.get_store().set_user_picture(
            UserPicture {
                user_id,
                mime_type,
                bytes,
                created_at: Some(now),
            },
            session,
        );
    }

    pub fn get_user_picture(&self, user_id: &str) -> Option<UserPicture> {
        let mut session = self.create_session();
        let result = self.get_user_picture_in_session(user_id, &mut session);
        let _ = session.rollback();
        result
    }

    pub fn get_user_picture_in_session(
        &self,
        user_id: &str,
        session: &mut DbSession,
    ) -> Option<UserPicture> {
        self.get_store().get_user_picture(user_id, session)
    }

    pub fn delete_user_picture(&self, user_id: &str) -> bool {
        let mut session = self.create_session();
        let result = self.delete_user_picture_in_session(user_id, &mut session);
        session.flush_and_commit().unwrap();
        result
    }

    pub fn delete_user_picture_in_session(&self, user_id: &str, session: &mut DbSession) -> bool {
        self.get_store().delete_user_picture(user_id, session)
    }

    pub fn save_group(&self, group: Group) {
        let mut session = self.create_session();
        self.save_group_in_session(group, &mut session);
        session.flush_and_commit().unwrap();
    }

    pub fn save_group_in_session(&self, group: Group, session: &mut DbSession) {
        self.get_store().insert_group(group, session);
    }

    pub fn find_group_by_id(&self, group_id: &str) -> Option<Group> {
        let mut session = self.create_session();
        let result = self.find_group_by_id_in_session(group_id, &mut session);
        let _ = session.rollback();
        result
    }

    pub fn find_group_by_id_in_session(
        &self,
        group_id: &str,
        session: &mut DbSession,
    ) -> Option<Group> {
        self.get_store().find_group(group_id, session)
    }

    pub fn delete_group(&self, group_id: &str) {
        let mut session = self.create_session();
        self.delete_group_in_session(group_id, &mut session);
        session.flush_and_commit().unwrap();
    }

    pub fn delete_group_in_session(&self, group_id: &str, session: &mut DbSession) {
        self.get_store().delete_group(group_id, session);
    }

    pub fn create_membership(&self, user_id: String, group_id: String) {
        let mut session = self.create_session();
        self.create_membership_in_session(user_id, group_id, &mut session);
        session.flush_and_commit().unwrap();
    }

    pub fn create_membership_in_session(
        &self,
        user_id: String,
        group_id: String,
        session: &mut DbSession,
    ) {
        self.get_store()
            .create_membership(user_id, group_id, session);
    }

    pub fn delete_membership(&self, user_id: &str, group_id: &str) {
        let mut session = self.create_session();
        self.delete_membership_in_session(user_id, group_id, &mut session);
        session.flush_and_commit().unwrap();
    }

    pub fn delete_membership_in_session(
        &self,
        user_id: &str,
        group_id: &str,
        session: &mut DbSession,
    ) {
        self.get_store()
            .delete_membership(user_id, group_id, session);
    }

    pub fn get_groups_by_user(&self, user_id: &str) -> Vec<Group> {
        let mut session = self.create_session();
        let result = self.get_groups_by_user_in_session(user_id, &mut session);
        let _ = session.rollback();
        result
    }

    pub fn get_groups_by_user_in_session(
        &self,
        user_id: &str,
        session: &mut DbSession,
    ) -> Vec<Group> {
        self.get_store().get_groups_by_user(user_id, session)
    }

    pub fn get_users_by_group(&self, group_id: &str) -> Vec<User> {
        let mut session = self.create_session();
        let result = self.get_users_by_group_in_session(group_id, &mut session);
        let _ = session.rollback();
        result
    }

    pub fn get_users_by_group_in_session(
        &self,
        group_id: &str,
        session: &mut DbSession,
    ) -> Vec<User> {
        self.get_store().get_users_by_group(group_id, session)
    }

    pub fn membership_exists(&self, user_id: &str, group_id: &str) -> bool {
        let mut session = self.create_session();
        let result = self.membership_exists_in_session(user_id, group_id, &mut session);
        let _ = session.rollback();
        result
    }

    pub fn membership_exists_in_session(
        &self,
        user_id: &str,
        group_id: &str,
        session: &mut DbSession,
    ) -> bool {
        self.get_store()
            .membership_exists(user_id, group_id, session)
    }

    pub fn list_memberships(&self) -> Vec<Membership> {
        let mut session = self.create_session();
        let result = self.list_memberships_in_session(&mut session);
        let _ = session.rollback();
        result
    }

    pub fn list_memberships_in_session(&self, session: &mut DbSession) -> Vec<Membership> {
        self.get_store().list_memberships(session)
    }

    pub fn create_user_query(&self) -> UserQuery {
        UserQuery::new(Arc::clone(&self.command_executor))
    }

    pub fn create_group_query(&self) -> GroupQuery {
        GroupQuery::new(Arc::clone(&self.command_executor))
    }

    pub fn create_token_query(&self) -> TokenQuery {
        TokenQuery::new(Arc::clone(&self.command_executor))
    }

    pub fn save_privilege(&self, privilege: Privilege) {
        let mut session = self.create_session();
        self.save_privilege_in_session(privilege, &mut session);
        session.flush_and_commit().unwrap();
    }

    pub fn save_privilege_in_session(&self, privilege: Privilege, session: &mut DbSession) {
        self.get_store().insert_privilege(privilege, session);
    }

    pub fn find_privilege_by_id(&self, privilege_id: &str) -> Option<Privilege> {
        let mut session = self.create_session();
        let result = self.find_privilege_by_id_in_session(privilege_id, &mut session);
        let _ = session.rollback();
        result
    }

    pub fn find_privilege_by_id_in_session(
        &self,
        privilege_id: &str,
        session: &mut DbSession,
    ) -> Option<Privilege> {
        self.get_store().find_privilege(privilege_id, session)
    }

    pub fn delete_privilege(&self, privilege_id: &str) {
        let mut session = self.create_session();
        self.delete_privilege_in_session(privilege_id, &mut session);
        session.flush_and_commit().unwrap();
    }

    pub fn delete_privilege_in_session(&self, privilege_id: &str, session: &mut DbSession) {
        self.get_store().delete_privilege(privilege_id, session);
    }

    pub fn add_user_privilege_mapping(&self, privilege_id: String, user_id: String) {
        let mut session = self.create_session();
        self.add_user_privilege_mapping_in_session(privilege_id, user_id, &mut session);
        session.flush_and_commit().unwrap();
    }

    pub fn add_user_privilege_mapping_in_session(
        &self,
        privilege_id: String,
        user_id: String,
        session: &mut DbSession,
    ) {
        let privilege = self
            .get_store()
            .find_privilege(&privilege_id, session)
            .unwrap_or_else(|| Privilege {
                id: privilege_id.clone(),
                name: privilege_id.clone(),
            });
        let mapping = PrivilegeMapping {
            id: uuid::Uuid::new_v4().to_string(),
            privilege_id: privilege.id,
            user_id: Some(user_id),
            group_id: None,
        };
        self.get_store().insert_privilege_mapping(mapping, session);
    }

    pub fn add_group_privilege_mapping(&self, privilege_id: String, group_id: String) {
        let mut session = self.create_session();
        self.add_group_privilege_mapping_in_session(privilege_id, group_id, &mut session);
        session.flush_and_commit().unwrap();
    }

    pub fn add_group_privilege_mapping_in_session(
        &self,
        privilege_id: String,
        group_id: String,
        session: &mut DbSession,
    ) {
        let privilege = self
            .get_store()
            .find_privilege(&privilege_id, session)
            .unwrap_or_else(|| Privilege {
                id: privilege_id.clone(),
                name: privilege_id.clone(),
            });
        let mapping = PrivilegeMapping {
            id: uuid::Uuid::new_v4().to_string(),
            privilege_id: privilege.id,
            user_id: None,
            group_id: Some(group_id),
        };
        self.get_store().insert_privilege_mapping(mapping, session);
    }

    pub fn delete_user_privilege_mapping(&self, privilege_id: &str, user_id: &str) {
        let mut session = self.create_session();
        self.delete_user_privilege_mapping_in_session(privilege_id, user_id, &mut session);
        session.flush_and_commit().unwrap();
    }

    pub fn delete_user_privilege_mapping_in_session(
        &self,
        privilege_id: &str,
        user_id: &str,
        session: &mut DbSession,
    ) {
        let mappings = self
            .get_store()
            .find_privilege_mappings_by_privilege(privilege_id, session);
        for mapping in mappings {
            if mapping.user_id.as_deref() == Some(user_id) {
                self.get_store()
                    .delete_privilege_mapping(&mapping.id, session);
            }
        }
    }

    pub fn delete_group_privilege_mapping(&self, privilege_id: &str, group_id: &str) {
        let mut session = self.create_session();
        self.delete_group_privilege_mapping_in_session(privilege_id, group_id, &mut session);
        session.flush_and_commit().unwrap();
    }

    pub fn delete_group_privilege_mapping_in_session(
        &self,
        privilege_id: &str,
        group_id: &str,
        session: &mut DbSession,
    ) {
        let mappings = self
            .get_store()
            .find_privilege_mappings_by_privilege(privilege_id, session);
        for mapping in mappings {
            if mapping.group_id.as_deref() == Some(group_id) {
                self.get_store()
                    .delete_privilege_mapping(&mapping.id, session);
            }
        }
    }

    pub fn get_privileges_for_user(&self, user_id: &str) -> Vec<Privilege> {
        let mut session = self.create_session();
        let result = self.get_privileges_for_user_in_session(user_id, &mut session);
        let _ = session.rollback();
        result
    }

    pub fn get_privileges_for_user_in_session(
        &self,
        user_id: &str,
        session: &mut DbSession,
    ) -> Vec<Privilege> {
        let store = self.get_store();
        let mut privileges = store
            .find_privilege_mappings_by_user(user_id, session)
            .into_iter()
            .filter_map(|mapping| store.find_privilege(&mapping.privilege_id, session))
            .collect::<Vec<_>>();

        for group in store.get_groups_by_user(user_id, session) {
            privileges.extend(
                store
                    .find_privilege_mappings_by_group(&group.id, session)
                    .into_iter()
                    .filter_map(|mapping| store.find_privilege(&mapping.privilege_id, session)),
            );
        }

        privileges.sort_by(|left, right| left.id.cmp(&right.id));
        privileges.dedup_by(|left, right| left.id == right.id);
        privileges
    }

    pub fn get_privileges_for_group(&self, group_id: &str) -> Vec<Privilege> {
        let mut session = self.create_session();
        let result = self.get_privileges_for_group_in_session(group_id, &mut session);
        let _ = session.rollback();
        result
    }

    pub fn get_privileges_for_group_in_session(
        &self,
        group_id: &str,
        session: &mut DbSession,
    ) -> Vec<Privilege> {
        let store = self.get_store();
        let mut privileges = store
            .find_privilege_mappings_by_group(group_id, session)
            .into_iter()
            .filter_map(|mapping| store.find_privilege(&mapping.privilege_id, session))
            .collect::<Vec<_>>();
        privileges.sort_by(|left, right| left.id.cmp(&right.id));
        privileges.dedup_by(|left, right| left.id == right.id);
        privileges
    }

    /// Privileges granted to the user *directly*, excluding anything inherited
    /// through group membership — Java's `createPrivilegeQuery().userId(id)`.
    ///
    /// Distinct from [`Self::get_privileges_for_user`], which unions the group
    /// grants because that is what an effective-permission check needs. Callers
    /// that manipulate mappings (deleting a user, listing what to revoke) need
    /// this narrower set.
    pub fn get_direct_privileges_for_user(&self, user_id: &str) -> Vec<Privilege> {
        let mut session = self.create_session();
        let result = self.get_direct_privileges_for_user_in_session(user_id, &mut session);
        let _ = session.rollback();
        result
    }

    pub fn get_direct_privileges_for_user_in_session(
        &self,
        user_id: &str,
        session: &mut DbSession,
    ) -> Vec<Privilege> {
        let store = self.get_store();
        let mut privileges = store
            .find_privilege_mappings_by_user(user_id, session)
            .into_iter()
            .filter_map(|mapping| store.find_privilege(&mapping.privilege_id, session))
            .collect::<Vec<_>>();
        privileges.sort_by(|left, right| left.id.cmp(&right.id));
        privileges.dedup_by(|left, right| left.id == right.id);
        privileges
    }

    /// Every privilege row. Java reaches these through
    /// `PrivilegeRepository.findAll()`, which the idm app's privilege list needs;
    /// there is no query builder for privileges because there is nothing to filter
    /// on.
    pub fn list_privileges(&self) -> Vec<Privilege> {
        let mut session = self.create_session();
        let result = self.list_privileges_in_session(&mut session);
        let _ = session.rollback();
        result
    }

    pub fn list_privileges_in_session(&self, session: &mut DbSession) -> Vec<Privilege> {
        let mut privileges = self.get_store().list_privileges(session);
        privileges.sort_by(|left, right| left.id.cmp(&right.id));
        privileges
    }

    /// The users and groups a privilege is granted to — the reverse of
    /// [`Self::get_privileges_for_user`]. Java exposes this as
    /// `PrivilegeMappingRepository.findByPrivilegeId`; the idm app's
    /// single-privilege screen lists both sides.
    ///
    /// A mapping row carries either `user_id` or `group_id`, never both, so the
    /// two returned vectors partition the mappings.
    pub fn get_privilege_mapping_ids(&self, privilege_id: &str) -> (Vec<String>, Vec<String>) {
        let mut session = self.create_session();
        let result = self.get_privilege_mapping_ids_in_session(privilege_id, &mut session);
        let _ = session.rollback();
        result
    }

    pub fn get_privilege_mapping_ids_in_session(
        &self,
        privilege_id: &str,
        session: &mut DbSession,
    ) -> (Vec<String>, Vec<String>) {
        let mappings = self
            .get_store()
            .find_privilege_mappings_by_privilege(privilege_id, session);

        let mut user_ids = Vec::new();
        let mut group_ids = Vec::new();
        for mapping in mappings {
            if let Some(user_id) = mapping.user_id {
                user_ids.push(user_id);
            } else if let Some(group_id) = mapping.group_id {
                group_ids.push(group_id);
            }
        }
        user_ids.sort();
        group_ids.sort();
        (user_ids, group_ids)
    }

    pub fn save_token(&self, token: Token) {
        let mut session = self.create_session();
        self.save_token_in_session(token, &mut session);
        session.flush_and_commit().unwrap();
    }

    pub fn save_token_in_session(&self, token: Token, session: &mut DbSession) {
        self.get_store().insert_token(token, session);
    }

    pub fn find_token_by_id(&self, token_id: &str) -> Option<Token> {
        let mut session = self.create_session();
        let result = self.find_token_by_id_in_session(token_id, &mut session);
        let _ = session.rollback();
        result
    }

    pub fn find_token_by_id_in_session(
        &self,
        token_id: &str,
        session: &mut DbSession,
    ) -> Option<Token> {
        self.get_store().find_token(token_id, session)
    }

    pub fn delete_token(&self, token_id: &str) {
        let mut session = self.create_session();
        self.delete_token_in_session(token_id, &mut session);
        session.flush_and_commit().unwrap();
    }

    pub fn delete_token_in_session(&self, token_id: &str, session: &mut DbSession) {
        self.get_store().delete_token(token_id, session);
    }
}

// ── User Query ──

pub struct UserQuery {
    state: QueryState<User>,
    first_name: Option<String>,
    last_name: Option<String>,
    email: Option<String>,
    member_of_group_id: Option<String>,
}

impl UserQuery {
    pub fn new(command_executor: Arc<DefaultCommandExecutor>) -> Self {
        Self {
            state: QueryState::new(command_executor),
            first_name: None,
            last_name: None,
            email: None,
            member_of_group_id: None,
        }
    }

    pub fn first_name(mut self, first_name: String) -> Self {
        self.first_name = Some(first_name);
        self
    }

    pub fn last_name(mut self, last_name: String) -> Self {
        self.last_name = Some(last_name);
        self
    }

    pub fn email(mut self, email: String) -> Self {
        self.email = Some(email);
        self
    }

    pub fn member_of_group_id(mut self, group_id: String) -> Self {
        self.member_of_group_id = Some(group_id);
        self
    }

    pub fn order_by_first_name(mut self) -> Self {
        self.state.order_by = Some("first_name".to_string());
        self
    }

    pub fn order_by_last_name(mut self) -> Self {
        self.state.order_by = Some("last_name".to_string());
        self
    }

    pub fn asc(mut self) -> Self {
        self.state.direction = Direction::Asc;
        self
    }

    pub fn desc(mut self) -> Self {
        self.state.direction = Direction::Desc;
        self
    }
}

pub struct UserQueryCmd {
    query: UserQuery,
}

impl UserQueryCmd {
    pub fn new(query: UserQuery) -> Self {
        Self { query }
    }
}

impl Command<Vec<User>> for UserQueryCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Vec<User>, crate::error::FlowableError> {
        let (store, session) = command_context.store_and_session();
        let mut users = if let Some(group_id) = &self.query.member_of_group_id {
            store.get_users_by_group(group_id, session)
        } else {
            store.list_users(session)
        };
        if let Some(first_name) = &self.query.first_name {
            users.retain(|u| u.first_name.as_deref() == Some(first_name));
        }
        if let Some(last_name) = &self.query.last_name {
            users.retain(|u| u.last_name.as_deref() == Some(last_name));
        }
        if let Some(email) = &self.query.email {
            users.retain(|u| u.email.as_deref() == Some(email));
        }
        if let Some(order_by) = &self.query.state.order_by {
            match order_by.as_str() {
                "first_name" => users.sort_by(|a, b| {
                    let ord = a.first_name.cmp(&b.first_name);
                    if matches!(self.query.state.direction, Direction::Desc) {
                        ord.reverse()
                    } else {
                        ord
                    }
                }),
                "last_name" => users.sort_by(|a, b| {
                    let ord = a.last_name.cmp(&b.last_name);
                    if matches!(self.query.state.direction, Direction::Desc) {
                        ord.reverse()
                    } else {
                        ord
                    }
                }),
                _ => {}
            }
        }
        Ok(users)
    }
}

impl Query<User, UserQuery> for UserQuery {
    fn list(&self) -> Result<Vec<User>, crate::error::FlowableError> {
        let query_clone = UserQuery {
            state: QueryState {
                command_executor: Arc::clone(&self.state.command_executor),
                phantom: std::marker::PhantomData,
                order_by: self.state.order_by.clone(),
                direction: match self.state.direction {
                    Direction::Asc => Direction::Asc,
                    Direction::Desc => Direction::Desc,
                },
            },
            first_name: self.first_name.clone(),
            last_name: self.last_name.clone(),
            email: self.email.clone(),
            member_of_group_id: self.member_of_group_id.clone(),
        };
        let cmd = UserQueryCmd::new(query_clone);
        self.state.command_executor.execute(&cmd)
    }

    fn single_result(&self) -> Result<Option<User>, crate::error::FlowableError> {
        let mut list = self.list()?;
        Ok(list.pop())
    }

    fn count(&self) -> Result<i64, crate::error::FlowableError> {
        let list = self.list()?;
        Ok(list.len() as i64)
    }
}

// ── Group Query ──

pub struct GroupQuery {
    state: QueryState<Group>,
    name: Option<String>,
    group_type: Option<String>,
    member_user_id: Option<String>,
}

impl GroupQuery {
    pub fn new(command_executor: Arc<DefaultCommandExecutor>) -> Self {
        Self {
            state: QueryState::new(command_executor),
            name: None,
            group_type: None,
            member_user_id: None,
        }
    }

    pub fn name(mut self, name: String) -> Self {
        self.name = Some(name);
        self
    }

    pub fn group_type(mut self, group_type: String) -> Self {
        self.group_type = Some(group_type);
        self
    }

    pub fn member_user_id(mut self, user_id: String) -> Self {
        self.member_user_id = Some(user_id);
        self
    }

    pub fn order_by_name(mut self) -> Self {
        self.state.order_by = Some("name".to_string());
        self
    }

    pub fn asc(mut self) -> Self {
        self.state.direction = Direction::Asc;
        self
    }

    pub fn desc(mut self) -> Self {
        self.state.direction = Direction::Desc;
        self
    }
}

pub struct GroupQueryCmd {
    query: GroupQuery,
}

impl GroupQueryCmd {
    pub fn new(query: GroupQuery) -> Self {
        Self { query }
    }
}

impl Command<Vec<Group>> for GroupQueryCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Vec<Group>, crate::error::FlowableError> {
        let (store, session) = command_context.store_and_session();
        let mut groups = if let Some(user_id) = &self.query.member_user_id {
            store.get_groups_by_user(user_id, session)
        } else {
            store.list_groups(session)
        };
        if let Some(name) = &self.query.name {
            groups.retain(|g| &g.name == name);
        }
        if let Some(group_type) = &self.query.group_type {
            groups.retain(|g| g.group_type.as_deref() == Some(group_type));
        }
        if let Some(order_by) = &self.query.state.order_by
            && order_by.as_str() == "name"
        {
            groups.sort_by(|a, b| {
                let ord = a.name.cmp(&b.name);
                if matches!(self.query.state.direction, Direction::Desc) {
                    ord.reverse()
                } else {
                    ord
                }
            });
        }
        Ok(groups)
    }
}

impl Query<Group, GroupQuery> for GroupQuery {
    fn list(&self) -> Result<Vec<Group>, crate::error::FlowableError> {
        let query_clone = GroupQuery {
            state: QueryState {
                command_executor: Arc::clone(&self.state.command_executor),
                phantom: std::marker::PhantomData,
                order_by: self.state.order_by.clone(),
                direction: match self.state.direction {
                    Direction::Asc => Direction::Asc,
                    Direction::Desc => Direction::Desc,
                },
            },
            name: self.name.clone(),
            group_type: self.group_type.clone(),
            member_user_id: self.member_user_id.clone(),
        };
        let cmd = GroupQueryCmd::new(query_clone);
        self.state.command_executor.execute(&cmd)
    }

    fn single_result(&self) -> Result<Option<Group>, crate::error::FlowableError> {
        let mut list = self.list()?;
        Ok(list.pop())
    }

    fn count(&self) -> Result<i64, crate::error::FlowableError> {
        let list = self.list()?;
        Ok(list.len() as i64)
    }
}

// ── Token Query ──

pub struct TokenQuery {
    state: QueryState<Token>,
    token_value: Option<String>,
    user_id: Option<String>,
}

impl TokenQuery {
    pub fn new(command_executor: Arc<DefaultCommandExecutor>) -> Self {
        Self {
            state: QueryState::new(command_executor),
            token_value: None,
            user_id: None,
        }
    }

    pub fn token_value(mut self, token_value: String) -> Self {
        self.token_value = Some(token_value);
        self
    }

    pub fn user_id(mut self, user_id: String) -> Self {
        self.user_id = Some(user_id);
        self
    }

    pub fn order_by_token_value(mut self) -> Self {
        self.state.order_by = Some("token_value".to_string());
        self
    }

    pub fn asc(mut self) -> Self {
        self.state.direction = Direction::Asc;
        self
    }

    pub fn desc(mut self) -> Self {
        self.state.direction = Direction::Desc;
        self
    }
}

pub struct TokenQueryCmd {
    query: TokenQuery,
}

impl TokenQueryCmd {
    pub fn new(query: TokenQuery) -> Self {
        Self { query }
    }
}

impl Command<Vec<Token>> for TokenQueryCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Vec<Token>, crate::error::FlowableError> {
        let (store, session) = command_context.store_and_session();
        let mut tokens = store.list_tokens(session);
        if let Some(token_value) = &self.query.token_value {
            tokens.retain(|t| &t.token_value == token_value);
        }
        if let Some(user_id) = &self.query.user_id {
            tokens.retain(|t| t.user_id.as_deref() == Some(user_id));
        }
        if let Some(order_by) = &self.query.state.order_by
            && order_by.as_str() == "token_value"
        {
            tokens.sort_by(|a, b| {
                let ord = a.token_value.cmp(&b.token_value);
                if matches!(self.query.state.direction, Direction::Desc) {
                    ord.reverse()
                } else {
                    ord
                }
            });
        }
        Ok(tokens)
    }
}

impl Query<Token, TokenQuery> for TokenQuery {
    fn list(&self) -> Result<Vec<Token>, crate::error::FlowableError> {
        let query_clone = TokenQuery {
            state: QueryState {
                command_executor: Arc::clone(&self.state.command_executor),
                phantom: std::marker::PhantomData,
                order_by: self.state.order_by.clone(),
                direction: match self.state.direction {
                    Direction::Asc => Direction::Asc,
                    Direction::Desc => Direction::Desc,
                },
            },
            token_value: self.token_value.clone(),
            user_id: self.user_id.clone(),
        };
        let cmd = TokenQueryCmd::new(query_clone);
        self.state.command_executor.execute(&cmd)
    }

    fn single_result(&self) -> Result<Option<Token>, crate::error::FlowableError> {
        let mut list = self.list()?;
        Ok(list.pop())
    }

    fn count(&self) -> Result<i64, crate::error::FlowableError> {
        let list = self.list()?;
        Ok(list.len() as i64)
    }
}
