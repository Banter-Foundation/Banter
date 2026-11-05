//! Client contracts for nested server-rail folders (BA-79).
//! Written before the rail UX — hex colors, context menus, and flatten order.

use banter::{
    context_menu_actions, context_menu_labels, context_menu_tone, folder_context_menu, parse_folder_color,
    rail_tree_rows, server_rail_context_menu, server_rail_item_menu, ContextMenuAction, ContextMenuTone,
    RailTreeRow, ServerFolder, ServerRail, ServerRailServer, ServerSummary,
};

#[test]
fn parse_folder_color_accepts_hex_and_rejects_junk() {
    assert_eq!(parse_folder_color(""), Ok(0));
    assert_eq!(parse_folder_color("   "), Ok(0));
    assert_eq!(parse_folder_color("5865F2"), Ok(0x5865F2));
    assert_eq!(parse_folder_color("#ed4245"), Ok(0xED4245));
    assert_eq!(parse_folder_color(" #57f287 "), Ok(0x57F287));
    assert!(parse_folder_color("12345").is_err());
    assert!(parse_folder_color("GGGGGG").is_err());
    assert!(parse_folder_color("#1234567").is_err());
    assert!(parse_folder_color("red").is_err());
}

#[test]
fn folder_and_server_rail_menus_cover_nest_rename_color_delete() {
    let folder = folder_context_menu(9, true);
    assert_eq!(
        context_menu_labels(&folder),
        [
            "Create Subfolder",
            "Rename Folder",
            "Change Color",
            "Delete Folder"
        ]
    );
    assert!(context_menu_actions(&folder).contains(&ContextMenuAction::CreateServerFolder {
        parent_id: Some(9)
    }));
    assert!(context_menu_actions(&folder).contains(&ContextMenuAction::RenameServerFolder { folder_id: 9 }));
    assert!(context_menu_actions(&folder).contains(&ContextMenuAction::RecolorServerFolder { folder_id: 9 }));
    assert!(context_menu_actions(&folder).contains(&ContextMenuAction::DeleteServerFolder { folder_id: 9 }));
    assert_eq!(
        context_menu_tone(&folder, "Delete Folder"),
        Some(ContextMenuTone::Danger)
    );

    let top = server_rail_context_menu();
    assert_eq!(context_menu_labels(&top), ["Create Folder"]);
    assert!(context_menu_actions(&top).contains(&ContextMenuAction::CreateServerFolder { parent_id: None }));

    let bare = server_rail_item_menu(4, None, &[]);
    assert_eq!(context_menu_labels(&bare), ["Plugins"]);
    assert!(context_menu_actions(&bare).contains(&ContextMenuAction::AddPlugin { server_id: 4 }));

    let in_folder = server_rail_item_menu(4, Some(9), &[("Games".into(), 9), ("FPS".into(), 12)]);
    assert!(context_menu_labels(&in_folder).contains(&"Remove from Folder"));
    assert!(context_menu_labels(&in_folder).contains(&"Plugins"));
    assert!(context_menu_actions(&in_folder).contains(&ContextMenuAction::MoveServerToFolder {
        server_id: 4,
        folder_id: None
    }));
    assert!(context_menu_actions(&in_folder).contains(&ContextMenuAction::MoveServerToFolder {
        server_id: 4,
        folder_id: Some(12)
    }));
}

#[test]
fn rail_tree_rows_nest_and_hide_collapsed_children() {
    let rail = ServerRail {
        folders: vec![
            ServerFolder {
                id: 1,
                parent_id: None,
                name: "Games".into(),
                color: 0x5865F2,
                position: 0,
                collapsed: false,
            },
            ServerFolder {
                id: 2,
                parent_id: Some(1),
                name: "FPS".into(),
                color: 0xED4245,
                position: 1,
                collapsed: true,
            },
        ],
        servers: vec![
            ServerRailServer {
                server: ServerSummary {
                    id: 10,
                    name: "Alpha".into(),
                    owner_id: 1,
                },
                folder_id: Some(1),
                position: 0,
            },
            ServerRailServer {
                server: ServerSummary {
                    id: 11,
                    name: "Beta".into(),
                    owner_id: 1,
                },
                folder_id: Some(2),
                position: 0,
            },
            ServerRailServer {
                server: ServerSummary {
                    id: 12,
                    name: "Gamma".into(),
                    owner_id: 1,
                },
                folder_id: None,
                position: 1,
            },
        ],
    };

    let rows = rail_tree_rows(&rail);
    assert_eq!(
        rows,
        vec![
            RailTreeRow::Folder { id: 1, depth: 0 },
            RailTreeRow::Server { id: 10, depth: 1 },
            RailTreeRow::Folder { id: 2, depth: 1 },
            // Beta is hidden because FPS is collapsed.
            RailTreeRow::Server { id: 12, depth: 0 },
        ]
    );

    let mut collapsed = rail.clone();
    collapsed.folders[0].collapsed = true;
    let hidden = rail_tree_rows(&collapsed);
    assert_eq!(
        hidden,
        vec![
            RailTreeRow::Folder { id: 1, depth: 0 },
            RailTreeRow::Server { id: 12, depth: 0 },
        ]
    );
}
