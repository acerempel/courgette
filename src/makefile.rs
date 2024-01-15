use eyre::Report;
use nom::bytes::complete::{is_not, tag};
use nom::combinator::{all_consuming, map};
use nom::error::Error;
use nom::multi::{many0, many0_count, separated_list0};
use nom::sequence::preceded;
use nom::{Finish, IResult};

type Thing = Vec<u8>;

pub struct Rule {
    targets: Vec<Thing>,
    prerequisites: Vec<Thing>,
    commands: Vec<Thing>,
}

fn rule(input: &[u8]) -> IResult<&[u8], Rule> {
    let path = map(is_not([0x20]), Vec::from);
    let (input, targets) = separated_list0(tag(" "), path)(input)?;
    let (input, _) = many0_count(tag(" "))(input)?;
    let (input, _) = tag(":")(input)?;
    let (input, _) = many0_count(tag(" "))(input)?;
    let (input, prerequisites) = separated_list0(tag(" "), path)(input)?;
    let (input, commands) = many0(preceded(tag("\n\t"), path))(input)?;
    let (input, _) = many0_count(tag("\n"))(input)?;
    let rule = Rule {
        targets,
        prerequisites,
        commands,
    };
    Ok((input, rule))
}

pub fn parse(input: &[u8]) -> Result<Vec<Rule>, Report> {
    Ok(all_consuming(many0(rule))(input)
        .finish()
        .map(|(_, rules)| rules)
        .map_err(|Error { input, code }| Error {
            input: String::from_utf8_lossy(input),
            code,
        })?)
}
