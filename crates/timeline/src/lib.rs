use std::{collections::HashMap, rc::Rc};

use chrono::{DateTime, Utc};
use uuid::Uuid;
use yewdux::prelude::*;

use identity::PeerId;

#[derive(Store, Default, PartialEq, Clone, Debug)]
pub struct Timelines {
    inner: HashMap<PeerId, Timeline>,
}

impl Timelines {
    fn post(&mut self, id: &PeerId, mut post: Post) {
        post.author = id.clone();

        let timeline = self.get_mut(id);
        timeline.history.insert(post.id.clone(), post);
    }

    fn get_mut(&mut self, id: &PeerId) -> &mut Timeline {
        self.inner.entry(id.clone()).or_insert_with(|| Timeline {
            author: id.clone(),
            history: Default::default(),
        })
    }
}

#[derive(PartialEq, Clone, Debug)]
pub struct Timeline {
    author: PeerId,
    history: HashMap<PostId, Post>,
}

#[derive(PartialEq, Eq, Clone, Debug, Hash)]
pub struct PostId(Rc<Uuid>);

#[derive(PartialEq, Clone, Debug)]
pub struct Post {
    id: PostId,
    author: PeerId,
    content: String,
    timestamp: DateTime<Utc>,
}

impl Post {
    pub fn new(author: PeerId, content: String) -> Self {
        Self {
            author,
            content,
            timestamp: Utc::now(),
            id: PostId(Uuid::new_v4().into()),
        }
    }
}

#[derive(PartialEq, Clone, Debug)]
pub enum Action {
    Post(PeerId, Post),
}

impl Reducer<Timelines> for Action {
    fn apply(self, mut timelines: Rc<Timelines>) -> Rc<Timelines> {
        let state = Rc::make_mut(&mut timelines);

        match self {
            Action::Post(id, post) => state.post(&id, post),
        }

        timelines
    }
}

#[cfg(test)]
mod tests {
    use identity::Identity;

    use super::*;

    #[test]
    fn post_assures_author_id() {
        let peer1 = Identity::new().as_peer();
        let peer2 = Identity::new().as_peer();
        let action = Action::Post(peer2.clone(), Post::new(peer1, "".into()));

        let timelines = action.apply(Rc::new(Default::default()));

        let post = timelines
            .inner
            .get(&peer2)
            .unwrap()
            .history
            .values()
            .next()
            .unwrap();

        assert_eq!(post.author, peer2);
    }
}
