//! Made-up session content: the spike has no server, keys or networking.

#[derive(Clone, PartialEq, Debug)]
pub struct Image {
    pub name: String,
    /// A `data:` URL; the real client would use an object URL for the
    /// decrypted blob.
    pub url: String,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Note {
    pub id: u32,
    pub author: &'static str,
    pub mine: bool,
    pub channel: String,
    pub text: String,
    pub reply_to: Option<u32>,
    pub images: Vec<Image>,
    pub edited: bool,
}

pub const ME: &str = "Ink Fox";
pub const PARTNER: &str = "Quiet Wren";

pub struct Channel {
    pub name: &'static str,
    pub about: &'static str,
}

pub const CHANNELS: [Channel; 3] = [
    Channel {
        name: "story",
        about: "The story itself",
    },
    Channel {
        name: "planning",
        about: "Where it's going, out of the story",
    },
    Channel {
        name: "world",
        about: "Places and people",
    },
];

pub fn notes() -> Vec<Note> {
    let n = |id, mine: bool, channel: &str, text: &str, reply_to| Note {
        id,
        author: if mine { ME } else { PARTNER },
        mine,
        channel: channel.into(),
        text: text.into(),
        reply_to,
        images: Vec::new(),
        edited: false,
    };
    vec![
        n(
            1,
            true,
            "story",
            "# The lighthouse ledger\n\nThe ledger's last page was still wet when Mara found it, the ink beaded like rain on the window behind her.",
            None,
        ),
        n(
            2,
            false,
            "story",
            "She didn't touch it. *Wet ink meant someone had been here an hour ago*, and the only way up was the stair she had just climbed.",
            None,
        ),
        n(
            3,
            true,
            "story",
            "> Tide at 4:10. Lamp lit. No ships.\n\nThe same line, in the same hand, on every page for forty years — except the last, which said **one ship**.",
            None,
        ),
        n(
            4,
            false,
            "planning",
            "Should the keeper be alive? I like it better if we never find out.",
            None,
        ),
        n(
            5,
            true,
            "planning",
            "Agreed. Let's keep it ambiguous until the storm chapter.",
            Some(4),
        ),
        n(
            6,
            false,
            "world",
            "The island: about a mile long, one jetty, the lighthouse at the north end. The village was abandoned in the `1920s`.",
            None,
        ),
    ]
}

/// About 20,000 words, for testing that typing stays smooth.
pub fn long_draft() -> String {
    let paragraph = "The wind came off the water in long cold breaths, and the lamp turned above her whether or not anyone watched it. She counted the steps again, *one hundred and twelve*, and found the same number she always did, which was **somehow worse**. ";
    let mut s = String::from("# A long draft\n\n");
    for i in 0..465 {
        if i % 25 == 0 {
            s.push_str(&format!("## Part {}\n\n", i / 25 + 1));
        }
        for _ in 0..1 {
            s.push_str(paragraph);
        }
        s.push_str("\n\n");
    }
    s
}
