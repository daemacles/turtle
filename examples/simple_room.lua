-- Minimal static-room reference world demonstrating locked doors, key pickup, and guard patrol.
-- Uses strict turn-based pacing and wide square tiles as a clean template for new puzzles.

--- World module exported to the host engine.
local world = {}

--- Display name shown in the top status bar.
world.title = "Simple Room"

--- Wide two-column tiles keep room proportions square.
world.cell_width = 2

--- Turn-based pacing advances strictly on player input.
world.pacing = { mode = "turn" }

--- Visual legend pairing room tiles and actor keys with ANSI colors.
world.palette = {
    [" "] = { "  ", "Black", "Black" },
    ["#"] = { "▒▒", "White", "BrightBlack" },
    ["."] = { " ·", "Green", "Black" },
    ["+"] = { "[]", "Yellow", "Red" },
    ["/"] = { "..", "Yellow", "Black" },
    ["X"] = { "<>", "BrightCyan", "Blue" },
    ["@"] = { "@ ", "BrightYellow" },
    ["K"] = { "k-", "BrightCyan" },
    ["M"] = { "M ", "BrightRed" },
}

--- Fixed static courtyard map centered around the origin.
local MAP = {
    "#############",
    "#...........#",
    "#...........#",
    "#.....#######",
    "#.....+.....#",
    "#...........#",
    "#.....#....X#",
    "#############",
}

local ORIGIN_X = -6
local ORIGIN_Y = -4

local has_key = false
local door_open = false
local escaped = false

--- Reads the static or mutated room tile at integer world coordinates.
---@param x integer Eastward grid coordinate.
---@param y integer Southward grid coordinate.
---@return string tile Palette key at the requested coordinate.
local function tile_at(x, y)
    local row_idx = y - ORIGIN_Y + 1
    local col_idx = x - ORIGIN_X + 1
    local row = MAP[row_idx]
    if not row or col_idx < 1 or col_idx > #row then
        return " "
    end
    local ch = row:sub(col_idx, col_idx)
    if ch == "+" and door_open then
        return "/"
    end
    return ch
end

--- Resets room doors and spawns the player, brass key, and patrolling sentinel.
---@param _seed integer Seed ignored by this deterministic static puzzle room.
---@param ecs any Scoped host entity-component world handle.
---@return table hud Initial status indicator and mission hint.
function world.init(_seed, ecs)
    has_key = false
    door_open = false
    escaped = false

    ecs:spawn({ name = "player", pos = { x = -4, y = -2 }, sprite = { tile = "@", layer = 10 }, player = true, solid = true })
    ecs:spawn({ name = "key", pos = { x = -4, y = 2 }, sprite = { tile = "K", layer = 2 }, item = "brass_key" })
    ecs:spawn({ name = "guard", pos = { x = 3, y = 1 }, sprite = { tile = "M", layer = 6 }, solid = true, patrol = { dir = 1 } })

    return {
        status = "KEY: NO",
        msg = "Find the brass key (k-) to unlock the red gate and reach <>.",
    }
end

--- Resolves player movement, door unlocking, shrine escape, and sentinel patrol.
---@param _t integer Current simulation step counter.
---@param requests table[] Player movement or wait intents for this turn.
---@param ecs any Scoped host entity-component world handle.
---@return table hud Updated key status and event message.
function world.step(_t, requests, ecs)
    local msg = nil
    local player_id = ecs:find_by_name("player")
    local player_pos = player_id and ecs:get(player_id, "pos") or nil
    local px = player_pos and player_pos.x or -4
    local py = player_pos and player_pos.y or -2

    local function find_at_with_component(tx, ty, comp)
        for _, eid in ipairs(ecs:at(tx, ty)) do
            if ecs:has(eid, comp) then
                return eid
            end
        end
        return nil
    end

    for _, req in ipairs(requests) do
        if player_id and req.id == player_id and req.action == "move" and (req.dx ~= 0 or req.dy ~= 0) then
            local nx, ny = px + req.dx, py + req.dy
            local dest = tile_at(nx, ny)
            local guard_hit = find_at_with_component(nx, ny, "patrol")

            if guard_hit then
                px, py = -4, -2
                ecs:set(player_id, "pos", { x = px, y = py })
                msg = "Sentinel bumped you back to the courtyard!"
            elseif dest == "+" then
                if has_key then
                    door_open = true
                    px, py = nx, ny
                    ecs:set(player_id, "pos", { x = px, y = py })
                    msg = "Unlocked the gate with the brass key!"
                else
                    msg = "The gate is locked. Grab the k- key south of you first."
                end
            elseif dest == "." or dest == "/" or dest == "X" then
                px, py = nx, ny
                ecs:set(player_id, "pos", { x = px, y = py })
                local key_id = find_at_with_component(nx, ny, "item")
                if key_id and ecs:get(key_id, "item") == "brass_key" then
                    has_key = true
                    ecs:despawn(key_id)
                    msg = "Picked up the brass key!"
                elseif dest == "X" and not escaped then
                    escaped = true
                    msg = "Reached the shrine! Press r to replay or edit this room live."
                end
            elseif dest == "#" then
                msg = "Cold stone wall."
            end
        end
    end

    for _, guard_id in ipairs(ecs:query("pos", "patrol")) do
        local gpos = ecs:get(guard_id, "pos")
        local patrol = ecs:get(guard_id, "patrol")
        if gpos and patrol then
            local gx = gpos.x + patrol.dir
            local g_tile = tile_at(gx, gpos.y)
            if g_tile == "#" or (gx == px and gpos.y == py) then
                ecs:set(guard_id, "patrol", { dir = -patrol.dir })
                if gx == px and gpos.y == py then
                    msg = "The sentinel blocks the eastern passage."
                end
            else
                ecs:set(guard_id, "pos", { x = gx, y = gpos.y })
            end
        end
    end

    local key_tag = escaped and "ESCAPED!" or (has_key and "KEY: YES" or "KEY: NO")
    return { status = key_tag, msg = msg }
end

--- Renders the requested world viewport bounds as a two-dimensional key grid.
---@param start_pos table Top-left corner containing integer `x` and `y` fields.
---@param end_pos table Bottom-right corner containing integer `x` and `y` fields.
---@param _t integer Unused simulation step counter.
---@return string[][] rows Row-major matrix of palette keys covering the viewport bounds.
function world.view(start_pos, end_pos, _t)
    local rows = {}
    local ry = 1
    for y = start_pos.y, end_pos.y do
        local row = {}
        local rx = 1
        for x = start_pos.x, end_pos.x do
            row[rx] = tile_at(x, y)
            rx = rx + 1
        end
        rows[ry] = row
        ry = ry + 1
    end
    return rows
end

return world
