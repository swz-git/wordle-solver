#[macro_use]
extern crate lazy_static;

use std::{collections::HashMap, error::Error, time::Duration};

use thirtyfour::{By, DesiredCapabilities, Key, WebDriver};
use tokio::time::{sleep, Instant};

lazy_static! {
    static ref WORDLIST: Vec<String> = include_str!("wordle_words.txt")
        .split("\n")
        .map(|x| x.to_owned())
        .collect();
}

async fn write_word(driver: &WebDriver, text: &str) -> Result<(), Box<dyn Error>> {
    let is_valid = WORDLIST.contains(&text.to_owned()); // <-- what?

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

fn apply_filter(target_array: &mut Vec<String>, rules: &HashMap<char, LetterState>) {
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

fn calculate_best_word(
    filtered_words: &Vec<String>,
    letters_state: &HashMap<char, LetterState>,
    guesses_made: &u8,
) -> String {
    let mut copy = WORDLIST.clone();
    let mut letter_occurances = calculate_letter_occurances(&copy);

    let closeness_score: i32 = letters_state.values().fold(0i32, |acc, state| match state {
        LetterState::Absent => 0,
        LetterState::Present(_) => 20,
        LetterState::Correct(_) => 40
    } + acc) - filtered_words.len() as i32;

    // if true, try best guess. if false, try to gather info
    let try_to_win = *guesses_made == 5
        || (6 - guesses_made) as usize >= filtered_words.len()
        || closeness_score > 0;

    println!(
        "closeness score: {}, possible words left are {}, will {}try to win",
        closeness_score,
        filtered_words.len(),
        if !try_to_win { "not " } else { "" }
    );

    // only valid guesses are good guesses if were trying to win
    if try_to_win {
        copy = filtered_words.clone();
        letter_occurances = calculate_letter_occurances(filtered_words);
    }

    copy.sort_by_cached_key(|x| {
        let score = x
            .chars()
            .fold(0, |acc, e| acc + letter_occurances.get(&e).unwrap()) as i32;
        score * -1
    });

    // if were not trying to win, sort the words by the chance that they reveal some useful info
    if !try_to_win {
        let mut letter_occurance_values = letter_occurances.values().collect::<Vec<&u32>>();
        letter_occurance_values.sort();
        let highest_letter_occurance = **letter_occurance_values.last().unwrap();

        copy.sort_by_cached_key(|x| {
            let mut i = 0;
            let score = x.chars().fold(0, |acc, e| {
                let letter_score = match letters_state.get(&e) {
                    Some(LetterState::Absent) => 0,
                    Some(LetterState::Correct(_)) => 1,
                    Some(LetterState::Present(pos)) => {
                        if *pos == i {
                            1
                        } else {
                            10
                        }
                    }
                    None => 100,
                };
                i += 1;
                letter_score
                    + (*letter_occurances.get(&e).unwrap() as f32 / highest_letter_occurance as f32
                        * 50f32)
                        .round() as i32
                    + acc
            });
            -score
        })
    }

    // sort words with duplicates further down
    copy.sort_by_cached_key(|x| {
        let mut char_count = HashMap::new();
        for c in x.chars() {
            *char_count.entry(c).or_insert(0) += 1;
        }
        char_count.values().filter(|&count| *count > 1).count()
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

    let mut filtered_words = WORDLIST.clone();

    let mut i = 0u8;
    loop {
        sleep(Duration::from_millis(1500)).await;

        let start_time = Instant::now();

        let letters_state = get_letters_state(&driver).await?;
        apply_filter(&mut filtered_words, &letters_state);

        let best_word = &calculate_best_word(&filtered_words, &letters_state, &i);

        let calc_time = start_time.elapsed();

        println!("it took {:?} to calculate word {}", calc_time, best_word);

        write_word(&driver, best_word).await?;

        if filtered_words.len() == 1 {
            break;
        }
        i += 1;
    }

    println!(
        "Found correct solution in {} guesses. Correct word was: {}",
        i, filtered_words[0]
    );

    Ok(())
}
