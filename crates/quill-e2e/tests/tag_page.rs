//! A tag type's tags with their prompt counts (`GET /tag-namespaces/{name}/tags`),
//! as the GUI's filter builder lists them (#26).

// Test code: panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use quill_core::Client;
use quill_e2e::TestServer;
use quill_proto::{
    api::CreatePersona,
    board::{CreatePrompt, Maturity, TagOrder, TagPageQuery},
};
use uuid::Uuid;

async fn persona(client: &Client, handle: &str) -> Uuid {
    client
        .create_persona(&CreatePersona {
            handle: handle.into(),
            display_name: None,
            summary: None,
        })
        .await
        .unwrap()
        .id
}

async fn post(client: &Client, persona: Uuid, tags: &[&str]) {
    client
        .create_prompt(&CreatePrompt {
            persona,
            title: format!("With {}", tags.join(" ")),
            body: "Body.".into(),
            maturity: Maturity::General,
            tags: tags.iter().map(|t| (*t).to_owned()).collect(),
            state: None,
        })
        .await
        .unwrap();
}

fn listed(page: &quill_proto::board::TagPage) -> Vec<(String, u64)> {
    page.items
        .iter()
        .map(|c| (c.tag.display.clone(), c.prompts))
        .collect()
}

#[tokio::test]
async fn a_tag_types_tags_come_with_counts_ordered_searched_and_paged() {
    let Some(server) = TestServer::start().await.unwrap() else {
        return;
    };
    let writer = server.account("ink").await.unwrap();
    let pen = persona(&writer, "ink_fox").await;
    post(&writer, pen, &["theme:grief", "theme:memory"]).await;
    post(&writer, pen, &["theme:grief"]).await;
    post(&writer, pen, &["theme:grief", "theme:isolation"]).await;
    post(&writer, pen, &["theme:memory"]).await;

    // Anyone can list them, signed in or not.
    let anyone = server.client();
    let page = |q: TagPageQuery| {
        let anyone = anyone.clone();
        async move { anyone.tag_page("theme", &q).await.unwrap() }
    };
    let used = page(TagPageQuery::default()).await;
    assert_eq!(
        listed(&used),
        [
            ("grief".to_owned(), 3),
            ("memory".to_owned(), 2),
            ("isolation".to_owned(), 1)
        ]
    );
    assert_eq!(used.total, 3);

    let by_name = page(TagPageQuery {
        order: Some(TagOrder::Name),
        ..TagPageQuery::default()
    })
    .await;
    assert_eq!(
        listed(&by_name)
            .into_iter()
            .map(|t| t.0)
            .collect::<Vec<_>>(),
        ["grief", "isolation", "memory"]
    );

    let letter = page(TagPageQuery {
        letter: Some('m'),
        ..TagPageQuery::default()
    })
    .await;
    assert_eq!(listed(&letter), [("memory".to_owned(), 2)]);

    let search = page(TagPageQuery {
        search: Some("RIE".into()),
        ..TagPageQuery::default()
    })
    .await;
    assert_eq!(listed(&search), [("grief".to_owned(), 3)]);

    // Paged: the total counts every match.
    let second = page(TagPageQuery {
        offset: Some(1),
        limit: Some(1),
        ..TagPageQuery::default()
    })
    .await;
    assert_eq!(listed(&second), [("memory".to_owned(), 2)]);
    assert_eq!(second.total, 3);

    // Tags no prompt carries yet are listed too, at 0.
    let genres = anyone
        .tag_page("genre", &TagPageQuery::default())
        .await
        .unwrap();
    assert!(genres.items.iter().all(|c| c.prompts == 0));
    assert!(genres.items.iter().any(|c| c.tag.display == "Horror"));

    // A type name that can't exist is refused; an unknown one is empty.
    assert!(
        anyone
            .tag_page("Not A Type!", &TagPageQuery::default())
            .await
            .is_err()
    );
    assert_eq!(
        anyone
            .tag_page("nonexistent", &TagPageQuery::default())
            .await
            .unwrap()
            .total,
        0
    );
    server.stop().await.unwrap();
}
