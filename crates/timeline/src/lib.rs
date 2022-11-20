use std::{collections::HashMap, rc::Rc};

use anyhow::ensure;
use chrono::{DateTime, Utc};
use uuid::Uuid;
use yewdux::prelude::*;

use identity::PeerId;

#[derive(Store, Default, PartialEq, Clone, Debug)]
pub struct Timelines {
    inner: HashMap<PeerId, Timeline>,
}

impl Timelines {
    fn create_post(&mut self, id: &PeerId, mut post: Post) -> anyhow::Result<()> {
        // Ensure the author is who they say they are.
        ensure!(post.author == id.clone());

        let timeline = self.get_mut(id);
        // Ensure it's impossible to overwrite existing posts.
        ensure!(!timeline.history.contains_key(&post.id));
        // Add new post
        post.timestamp = Utc::now();
        timeline.history.insert(post.id.clone(), post);

        Ok(())
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
    CreatePost(PeerId, Post),
}

impl Reducer<Timelines> for Action {
    fn apply(self, mut timelines: Rc<Timelines>) -> Rc<Timelines> {
        let state = Rc::make_mut(&mut timelines);

        match self {
            Action::CreatePost(id, post) => {
                state.create_post(&id, post).ok();
            }
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
        let action = Action::CreatePost(peer2.clone(), Post::new(peer1, "".into()));

        let timelines = action.apply(Rc::new(Default::default()));

        let tl = timelines.inner.get(&peer2);

        assert!(tl.is_none());
    }

    #[test]
    fn post_does_not_overwrite() {
        let id = Identity::new().as_peer();

        let mut post = Post::new(id.clone(), "".into());
        let t1 = Action::CreatePost(id.clone(), post.clone()).apply(Rc::new(Default::default()));
        post.content = "some new data".into();
        let t2 = Action::CreatePost(id, post).apply(t1.clone());

        assert_eq!(t1, t2);
    }

    #[test]
    fn post_timestamp_is_updated() {
        let id = Identity::new().as_peer();

        let mut post = Post::new(id.clone(), "".into());
        let t1 = Utc::now() - chrono::Duration::days(1);
        post.timestamp = t1;
        let timeline = Action::CreatePost(id.clone(), post).apply(Rc::new(Default::default()));

        let t2 = timeline
            .inner
            .get(&id)
            .unwrap()
            .history
            .values()
            .next()
            .unwrap()
            .timestamp;

        assert_ne!(t2, t1);
        // assert_eq!(t2, Utc::now());
    }
}
