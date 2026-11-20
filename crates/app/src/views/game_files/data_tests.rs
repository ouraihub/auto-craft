use super::*;

#[test]
fn unity_bundle_same_ids_export_the_selected_serialized_file() {
    let (archives, truncated) = archives(json!({"folder":"C:/game","engine":"unity","truncated":false,"archives":[
        {"path":"C:/game/data.unity3d","name":"data.unity3d","kind":"unity_bundle","size":400,"list_tool":"game_unity_bundle_list"}
    ]})).unwrap();
    assert!(!truncated);
    let archive = &archives[0];
    assert!(archive.supports_pictures());
    let (tool, _) = archive.listing(0, "Wall").unwrap();
    assert_eq!(tool, "game_unity_list");
    let rows = page(
        archive,
        json!({"format_version":22,"matching":3,"offset":0,"objects":[
            {"file":"CAB-a","path_id":-42,"class":"Texture2D","name":"Icon","size":88},
            {"file":"CAB-b","path_id":-42,"class":"Texture2D","name":"Wall","size":96},
            {"file":"CAB-b","path_id":9,"class":"AudioClip","name":null,"size":20}
        ]}),
        0,
    )
    .unwrap();
    assert_eq!(rows.items[1].name, "Wall");
    let picture = rows.items[1].picture.as_ref().unwrap();
    let (_, params) = picture.export(archive, Path::new("C:/out.png"));
    assert_eq!(params["file"], "CAB-b");
    assert_eq!(params["path_id"], -42);
    assert!(rows.items[2].picture.is_none());
    assert_eq!(rows.items[2].name, "#9");
}
#[test]
fn pak_candidates_do_not_treat_sidecars_as_textures_and_keep_paging() {
    let archive = Archive {
        path: "C:/game.pak".into(),
        name: "game.pak".into(),
        kind: "unreal_pak".into(),
        size: 4,
        list_tool: "game_pak_list".into(),
    };
    let rows = page(
        &archive,
        json!({"offset":400,"matching":403,"files":[
            {"path":"Game/UI/icon.UASSET","size":12}, {"path":"Game/UI/icon.uexp","size":20}
        ]}),
        400,
    )
    .unwrap();
    assert!(matches!(
        rows.items[0].picture,
        Some(Picture::Unreal { .. })
    ));
    assert!(rows.items[1].picture.is_none());
    assert_eq!(rows.next, Some(402));
    assert!(page(&archive, json!({"offset":0,"matching":403,"files":[]}), 400).is_err());
}
#[test]
fn godot_fallback_uses_entries_and_never_offers_texture_conversion() {
    let (archives, _) = archives(json!({"truncated":false,"archives":[
        {"path":"C:/game.exe","name":"game.exe","kind":"godot_pck","size":400,"list_tool":"core_packed_files_list","declared_files":2}
    ]})).unwrap();
    let rows = page(&archives[0], json!({"format":"godot_pck","matching":2,"entries":[
        {"path":"images/icon.png","offset":12,"stored_size":3,"size":3,"compression":"none","kind":null},
        {"path":"scenes/main.tscn","offset":15,"stored_size":4,"size":null,"compression":"none","kind":null}
    ]}), 0).unwrap();
    assert_eq!(rows.items[0].name, "images/icon.png");
    assert_eq!(rows.items[1].size, None);
    assert!(!archives[0].supports_pictures());
    assert!(rows.items.iter().all(|r| r.picture.is_none()));
}
#[test]
fn iostore_unnamed_chunks_are_a_valid_empty_listing() {
    let archive = Archive {
        path: "C:/game.utoc".into(),
        name: "game.utoc".into(),
        kind: "unreal_iostore".into(),
        size: 8,
        list_tool: "game_iostore_list".into(),
    };
    let rows = page(
        &archive,
        json!({"chunk_count":120,"matching":null,"offset":0,"files":[]}),
        0,
    )
    .unwrap();
    assert!(rows.items.is_empty());
    assert_eq!(rows.next, None);
    assert!(!archive.supports_pictures());
}
#[test]
fn malformed_rows_are_errors_not_successful_empty_archives() {
    assert!(archives(json!({"archives":[{"name":"missing identity"}],"truncated":false})).is_err());
    assert!(archives(json!({"summary":"not a tool result"})).is_err());
    let archive = Archive {
        path: "C:/game.assets".into(),
        name: "game.assets".into(),
        kind: "unity_assets".into(),
        size: 8,
        list_tool: "game_unity_list".into(),
    };
    assert!(page(&archive, json!({"offset":0,"matching":1,"objects":[{"class":"Texture2D","name":"lost id","size":5}]}), 0).is_err());
}
#[test]
fn export_preview_uses_saved_mip_dimensions_and_selected_output_only() {
    let value = json!({"output":"C:/out.png","width":1024,"height":1024,"saved":{"width":128,"height":64,"mip":3,"from":"ubulk"},"size":42,"sha256":"fixture"});
    let picture = saved(value.clone(), Path::new("C:/out.png")).unwrap();
    assert_eq!(picture.dimensions(), (128, 64));
    assert!(saved(value, Path::new("C:/other.png")).is_err());
    assert!(
        saved(
            json!({"output":"C:/out.png","width":0,"height":1}),
            Path::new("C:/out.png")
        )
        .is_err()
    );
}
#[test]
fn evidence_result_and_destination_name_do_not_lose_the_selected_picture() {
    let reply = ToolReply::Evidence {
        id: None,
        body: json!({"result":{"output":"C:/out.png","width":2,"height":3}}),
    };
    assert_eq!(
        saved(result(reply), Path::new("C:/out.png"))
            .unwrap()
            .dimensions(),
        (2, 3)
    );
    assert_eq!(suggested_name("Game/Content/UI/icon.uasset"), "icon.png");
    assert_eq!(suggested_name("folder\\weird:name"), "weird_name.png");
}
#[test]
fn unknown_kind_preserves_bundle_node_browsing_without_picture_exports() {
    let (archives, _) = archives(json!({"truncated":false,"archives":[
        {"path":"C:/game/other.bundle","name":"other.bundle","kind":"other_archive","size":400,"list_tool":"game_unity_bundle_list"}
    ]})).unwrap();
    let archive = &archives[0];
    assert_eq!(archive.listing(0, "").unwrap().0, "game_unity_bundle_list");
    let rows = page(
        archive,
        json!({"matching":1,"offset":0,"files":[
            {"path":"CAB-a","size":88,"serialized":true,"objects":{"object_count":2}}
        ]}),
        0,
    )
    .unwrap();
    assert_eq!(rows.items[0].name, "CAB-a");
    assert!(rows.items[0].picture.is_none());
    assert!(!archive.supports_pictures());
}
