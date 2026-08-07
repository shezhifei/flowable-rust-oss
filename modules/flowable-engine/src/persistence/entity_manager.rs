use crate::persistence::db_session::DbSession;

pub trait EntityManager<T> {
    fn insert(&mut self, entity: &T, session: &mut DbSession);
    fn update(&mut self, entity: &T, session: &mut DbSession);
    fn delete(&mut self, id: &str, session: &mut DbSession);
    fn find_by_id(&mut self, id: &str, session: &mut DbSession) -> Option<T>;
}
