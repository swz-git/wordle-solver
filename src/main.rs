#[macro_use]
extern crate lazy_static;

use std::{any::Any, collections::HashMap, error::Error, future, time::Duration};

use futures::future::join_all;
use regex::Regex;
use thirtyfour::{
    actions::{ActionSequence, KeyActions},
    By, DesiredCapabilities, Key, WebDriver,
};
use tokio::{
    io::{self, AsyncReadExt},
    process::Command,
    time::{sleep, Instant},
};

lazy_static! {
    static ref wordlist: Vec<String> = include_str!("wordle_words.txt")
        .split("\n")
        .map(|x| x.to_owned())
        .collect();
}

async fn write_word(driver: &WebDriver, text: &str) -> Result<(), Box<dyn Error>> {
    let is_valid = wordlist.contains(&text.to_owned()); // <-- what?

    if !is_valid {
        return Err("Input is not in wordle word list".into());
    };

    driver
        .action_chain()
        .send_keys(text)
        .key_down(Key::Return)
        .key_up(Key::Return)
        .perform()
        .await?;

    sleep(Duration::from_millis(500)).await;

    Ok(())
}

#[derive(Debug)]
enum LetterState {
    /// Gray
    Absent,
    /// Yellow
    Present(u8),
    /// Green
    Correct(u8),
}

async fn get_letters_state(
    driver: &WebDriver,
) -> Result<HashMap<char, LetterState>, Box<dyn Error>> {
    let mut map: HashMap<char, LetterState> = HashMap::new();

    let known_letters_elems = driver.find_all(By::Css("div[data-state]")).await?;

    for elem in known_letters_elems {
        if elem.attr("data-state").await?.unwrap() == "empty" {
            break;
        }

        let ch = elem
            .attr("aria-label")
            .await?
            .unwrap()
            .chars()
            .collect::<Vec<char>>()[0];
        let pos: u8 = elem
            .find(By::XPath("./.."))
            .await?
            .attr("style")
            .await?
            .unwrap()
            .chars()
            .collect::<Vec<char>>()[17]
            .to_digit(10)
            .unwrap() as u8;
        let state = match elem.attr("data-state").await?.unwrap().as_str() {
            "absent" => LetterState::Absent,
            "present" => LetterState::Present(pos),
            "correct" => LetterState::Correct(pos),
            _ => return Err("invalid data-state".into()),
        };
        map.insert(ch, state);
    }

    Ok(map)
}

fn apply_filter(target_array: &mut Vec<String>, rules: HashMap<char, LetterState>) {
    for rule in rules.iter() {
        match rule.1 {
            LetterState::Absent => target_array.retain(|x| !x.contains(&rule.0.to_string())),
            LetterState::Present(pos) => target_array.retain(|x| {
                x.contains(&rule.0.to_string()) && x.chars().nth(*pos as usize).unwrap() != *rule.0
            }),
            LetterState::Correct(pos) => {
                target_array.retain(|x| x.chars().nth(*pos as usize).unwrap() == *rule.0)
            }
        }
    }
}

fn calculate_letter_occurances(array_ref: &Vec<String>) -> HashMap<char, u32> {
    let mut map: HashMap<char, u32> = HashMap::new();
    for word in array_ref {
        for c in word.chars() {
            if map.contains_key(&c) {
                map.insert(c, map.get(&c).unwrap() + 1);
            } else {
                map.insert(c, 1);
            }
        }
    }
    map
}

fn calculate_best_word(array_ref: &Vec<String>) -> String {
    let mut copy = array_ref.clone();
    let letter_occurances = calculate_letter_occurances(array_ref);
    copy.sort_by_cached_key(|x| {
        let score = x
            .chars()
            .fold(0, |acc, e| acc + letter_occurances.get(&e).unwrap()) as i32;
        score * -1
    });
    copy.sort_by(|a, b| {
        fn count_duplicates(s: &str) -> usize {
            let mut char_count = HashMap::new();
            for c in s.chars() {
                *char_count.entry(c).or_insert(0) += 1;
            }
            char_count.values().filter(|&count| *count > 1).count()
        }
        count_duplicates(a)
            .partial_cmp(&count_duplicates(b))
            .unwrap()
    });
    copy[0].clone()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let caps = DesiredCapabilities::firefox();
    let driver = WebDriver::new("http://localhost:4444", caps).await?;

    driver
        .goto("https://www.nytimes.com/games/wordle/index.html")
        .await?;

    // accept cookies
    driver
        .find(By::Id("pz-gdpr-btn-accept"))
        .await?
        .click()
        .await?;

    // close popup
    driver
        .find(By::ClassName("Welcome-module_button__ZG0Zh"))
        .await?
        .click()
        .await?;

    // close popup
    driver
        .find(By::ClassName("Modal-module_closeIcon__TcEKb"))
        .await?
        .click()
        .await?;

    let mut filtered_words = wordlist.clone();

    loop {
        sleep(Duration::from_millis(1500)).await;
        let start_time = Instant::now();
        apply_filter(&mut filtered_words, get_letters_state(&driver).await?);
        let best_word = &calculate_best_word(&filtered_words);
        let calc_time = start_time.elapsed();
        write_word(&driver, best_word).await?;
        println!("it took {:?} to calculate word {}", calc_time, best_word);
        if filtered_words.len() == 1 {
            break;
        }
    }

    println!("Correct word was: {}", filtered_words[0]);

    Ok(())
}
