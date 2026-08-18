//! Cross-platform enumeration of executable names for the preset target picker.

use std::collections::HashMap;

pub fn list_running_processes() -> Result<Vec<String>, String> {
    platform::list_running_processes().map(unique_process_names)
}

fn unique_process_names(names: Vec<String>) -> Vec<String> {
    let mut unique = HashMap::new();
    for name in names {
        let trimmed = name.trim();
        if !trimmed.is_empty() {
            unique
                .entry(trimmed.to_lowercase())
                .or_insert_with(|| trimmed.to_owned());
        }
    }

    let mut names: Vec<_> = unique.into_values().collect();
    names.sort_by_key(|name| name.to_lowercase());
    names
}

#[cfg(target_os = "linux")]
mod platform {
    use std::{collections::HashMap, fs, os::unix::fs::MetadataExt, path::Path};

    #[derive(Debug)]
    struct Candidate {
        pid: u32,
        name: String,
    }

    pub fn list_running_processes() -> Result<Vec<String>, String> {
        let current_uid = fs::metadata("/proc/self")
            .map_err(|error| format!("could not inspect the current process: {error}"))?
            .uid();
        let entries = fs::read_dir("/proc")
            .map_err(|error| format!("could not enumerate running processes: {error}"))?;
        let mut groups: HashMap<String, Vec<Candidate>> = HashMap::new();

        for entry in entries.flatten() {
            let file_name = entry.file_name();
            if !file_name.as_encoded_bytes().iter().all(u8::is_ascii_digit) {
                continue;
            }
            let Some(pid) = file_name.to_string_lossy().parse::<u32>().ok() else {
                continue;
            };
            if entry.metadata().map(|metadata| metadata.uid()).ok() != Some(current_uid) {
                continue;
            }

            let process_path = entry.path();
            let Some(group) = desktop_application_group(&process_path) else {
                continue;
            };
            let Some(name) = executable_name(&process_path) else {
                continue;
            };
            groups
                .entry(group)
                .or_default()
                .push(Candidate { pid, name });
        }

        Ok(groups
            .into_iter()
            .filter_map(|(group, candidates)| representative_name(&group, candidates))
            .collect())
    }

    fn executable_name(process_path: &Path) -> Option<String> {
        fs::read_link(process_path.join("exe"))
            .ok()
            .and_then(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .or_else(|| {
                fs::read_to_string(process_path.join("comm"))
                    .ok()
                    .map(|name| name.trim().to_owned())
            })
            .filter(|name| !name.is_empty())
    }

    fn desktop_application_group(process_path: &Path) -> Option<String> {
        let cgroup = fs::read_to_string(process_path.join("cgroup")).ok()?;
        desktop_application_group_from_cgroup(&cgroup)
    }

    fn desktop_application_group_from_cgroup(cgroup: &str) -> Option<String> {
        cgroup.lines().find_map(|line| {
            let (_, path) = line.split_once("::")?;
            let application = path.split_once("/app.slice/")?.1;
            let mut components = application.split('/');
            let group = components.next()?;
            let child = components.next();

            if !group.starts_with("app-")
                || !(group.ends_with(".scope") || group.ends_with(".service"))
                || !matches!(child, None | Some("main.scope"))
                || components.next().is_some()
            {
                return None;
            }

            Some(group.to_owned())
        })
    }

    fn representative_name(group: &str, candidates: Vec<Candidate>) -> Option<String> {
        let group_lowercase = group.to_lowercase();
        let matching_candidates: Vec<_> = candidates
            .iter()
            .filter(|candidate| {
                let hint = application_hint(&candidate.name);
                hint.len() >= 3 && group_lowercase.contains(hint)
            })
            .collect();

        if !matching_candidates.is_empty() {
            return most_frequent_name(matching_candidates.into_iter());
        }

        if let Some(group_pid) = group_process_id(group) {
            if let Some(candidate) = candidates
                .iter()
                .find(|candidate| candidate.pid == group_pid)
            {
                return Some(candidate.name.clone());
            }
        }

        let application_candidates: Vec<_> = candidates
            .iter()
            .filter(|candidate| !is_desktop_helper(&candidate.name))
            .collect();
        if !application_candidates.is_empty() {
            return most_frequent_name(application_candidates.into_iter());
        }

        most_frequent_name(candidates.iter())
    }

    fn most_frequent_name<'a>(candidates: impl Iterator<Item = &'a Candidate>) -> Option<String> {
        let mut frequency: HashMap<String, (usize, String)> = HashMap::new();
        for candidate in candidates {
            let entry = frequency
                .entry(candidate.name.to_lowercase())
                .or_insert_with(|| (0, candidate.name.clone()));
            entry.0 += 1;
        }

        frequency
            .into_values()
            .max_by(|left, right| left.0.cmp(&right.0).then_with(|| right.1.cmp(&left.1)))
            .map(|(_, name)| name)
    }

    fn application_hint(name: &str) -> &str {
        let lowercase = name.to_lowercase();
        let suffix_length = [".appimage", ".exe", ".bin", "-bin"]
            .into_iter()
            .find_map(|suffix| lowercase.ends_with(suffix).then_some(suffix.len()))
            .unwrap_or_default();
        &name[..name.len() - suffix_length]
    }

    fn is_desktop_helper(name: &str) -> bool {
        matches!(
            name.to_lowercase().as_str(),
            "bwrap" | "xdg-dbus-proxy" | "sd_dummy" | "chrome_crashpad_handler"
        )
    }

    fn group_process_id(group: &str) -> Option<u32> {
        group
            .trim_end_matches(".scope")
            .rsplit('-')
            .next()?
            .parse()
            .ok()
    }

    #[cfg(test)]
    mod tests {
        use super::{desktop_application_group_from_cgroup, representative_name, Candidate};

        #[test]
        fn desktop_scopes_are_kept_but_terminal_tabs_and_services_are_not() {
            assert_eq!(
                desktop_application_group_from_cgroup(
                    "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-flatpak-org.mozilla.firefox-42.scope",
                ),
                Some("app-flatpak-org.mozilla.firefox-42.scope".to_owned())
            );
            assert_eq!(
                desktop_application_group_from_cgroup(
                    "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-org.kde.konsole-10.scope/tab(11).scope",
                ),
                None
            );
            assert_eq!(
                desktop_application_group_from_cgroup(
                    "0::/user.slice/user-1000.slice/user@1000.service/session.slice/plasma-kwin_wayland.service",
                ),
                None
            );
        }

        #[test]
        fn application_name_wins_over_helpers_in_the_same_scope() {
            let candidates = vec![
                Candidate {
                    pid: 10,
                    name: "bwrap".to_owned(),
                },
                Candidate {
                    pid: 11,
                    name: "firefox-bin".to_owned(),
                },
                Candidate {
                    pid: 12,
                    name: "sd_dummy".to_owned(),
                },
            ];
            assert_eq!(
                representative_name("app-flatpak-org.mozilla.firefox-42.scope", candidates),
                Some("firefox-bin".to_owned())
            );
        }

        #[test]
        fn unrelated_background_helpers_do_not_represent_an_application() {
            let candidates = vec![
                Candidate {
                    pid: 11,
                    name: "sd_dummy".to_owned(),
                },
                Candidate {
                    pid: 12,
                    name: "Vesktop.AppImage".to_owned(),
                },
            ];
            assert_eq!(
                representative_name("app-org.chromium.Chromium-10.scope", candidates),
                Some("Vesktop.AppImage".to_owned())
            );
        }
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use std::{collections::HashSet, mem::size_of};

    use windows::Win32::{
        Foundation::{CloseHandle, BOOL, HANDLE, HWND, LPARAM},
        System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        },
        UI::WindowsAndMessaging::{
            EnumWindows, GetWindowTextLengthW, GetWindowThreadProcessId, IsWindowVisible,
        },
    };

    struct Snapshot(HANDLE);

    impl Drop for Snapshot {
        fn drop(&mut self) {
            // SAFETY: this handle was returned by CreateToolhelp32Snapshot and is owned here.
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    pub fn list_running_processes() -> Result<Vec<String>, String> {
        // SAFETY: the snapshot and entry remain valid for the duration of enumeration.
        unsafe {
            let mut visible_processes = HashSet::new();
            let _ = EnumWindows(
                Some(collect_visible_process),
                LPARAM((&mut visible_processes as *mut HashSet<u32>) as isize),
            );
            let snapshot = Snapshot(
                CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)
                    .map_err(|error| format!("could not enumerate running processes: {error}"))?,
            );
            let mut entry = PROCESSENTRY32W {
                dwSize: size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };
            let mut names = Vec::new();

            if Process32FirstW(snapshot.0, &mut entry).as_bool() {
                loop {
                    let length = entry
                        .szExeFile
                        .iter()
                        .position(|character| *character == 0)
                        .unwrap_or(entry.szExeFile.len());
                    if visible_processes.contains(&entry.th32ProcessID) {
                        names.push(String::from_utf16_lossy(&entry.szExeFile[..length]));
                    }

                    if !Process32NextW(snapshot.0, &mut entry).as_bool() {
                        break;
                    }
                }
            }

            Ok(names)
        }
    }

    unsafe extern "system" fn collect_visible_process(window: HWND, state: LPARAM) -> BOOL {
        if IsWindowVisible(window).as_bool() && GetWindowTextLengthW(window) > 0 {
            let mut process_id = 0;
            GetWindowThreadProcessId(window, Some(&mut process_id));
            if process_id != 0 {
                let processes = &mut *(state.0 as *mut HashSet<u32>);
                processes.insert(process_id);
            }
        }
        BOOL(1)
    }
}

#[cfg(test)]
mod tests {
    use super::unique_process_names;

    #[test]
    fn process_names_are_trimmed_deduplicated_and_sorted() {
        assert_eq!(
            unique_process_names(vec![
                " spotify ".to_owned(),
                "Firefox".to_owned(),
                "SPOTIFY".to_owned(),
                String::new(),
            ]),
            vec!["Firefox", "spotify"]
        );
    }
}
