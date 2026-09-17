[x]  we need to make it so the user input area can be multiline supported with ctrl+j to add a next line like many other tuis use etc...

[x] user input when a user types beyound the input width the text doesn't wrap we should wrap the text (not by word boundry like we do else where)

[ ] Thinking, Agent output, read write edit output rendering see ../rho for examples we need to render ouput in nice clean way usually markdown rendering of things looks really nice and good, at least we need to have line breaks where the agent has line breaks in the output, but ideally we have some nice syntek rendering of things

[ ] can we add the current model context into the footer areas and how much is used up as well? inlcuding the current model being used, other things in the future will be `↑3M ↓116k R58M $1.832 30.3%/1M @1531.3t/s` tokens in and out R and W cached context%left/context size token rate and provider quotas `89% 4h33m 65% 5d18h` in the 5hour and 7day outputs. some of this is done in ../rho we can refrence we should make it better

[x] move providers up a level so that providers can be a module of many providers example we might need to use antigravity and that provider might need to handle the oauth workflow, and things like that look at `pi-antigravity` plugin, or we might need to use claude oauth modles look at the `benvargas/pi-claude-code-use` this one gets even more advanced because of the header changes that need to take place, some of this is done in ../rho but we should make it better

[ ] - **The 806-line `app/mod.rs`.** Your own rule says "small & modular, <300 lines, too much LOC is a smell" — but `too_many_lines` only lints *functions*, so that file is flying under the rule while violating its spirit. Is that state that legitimately won't split, or the kind of thing you'd break into `app::state` / `app::modes` / `app::feed`? Curious if it's deliberate.

[ ] - copy with enter return to the user input but doesn't auto scroll to the end
