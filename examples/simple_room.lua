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
local guard_dir = 1

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

--- Resets room doors and spawns player, brass key, and patrolling sentinel.
---@param _seed integer Seed ignored by this deterministic static puzzle room.
---@return table[] entities Starting player, key, and sentinel entities.
---@return table hud Initial status indicator and mission hint.
function world.init(_seed)
    has_key = false
    door_open = false
    escaped = false
    guard_dir = 1

    local entities = {
        { id = "player", x = -4, y = -2, tile = "@", player = true },
        { id = "key", x = -4, y = 2, tile = "K" },
        { id = "guard", x = 3, y = 1, tile = "M" },
    }
    return entities, {
        status = "KEY: NO",
        msg = "Find the brass key (k-) to unlock the red gate and reach <>.",
    }
end

--- Resolves player movement, door unlocking, shrine escape, and sentinel patrol.
---@param _t integer Current simulation step counter.
---@param requests table[] Player movement or wait intents for this turn.
---@param entities table<string, table> Snapshot of live entities keyed by identifier.
---@return table[] patches Entity position or removal updates produced by this turn.
---@return table hud Updated key status and event message.
function world.step(_t, requests, entities)
    local patches = {}
    local msg = nil
    local player = entities["player"]
    local px = player and player.x or -4
    local py = player and player.y or -2

    for _, req in ipairs(requests) do
        if req.id == "player" and req.action == "move" and (req.dx ~= 0 or req.dy ~= 0) then
            local nx, ny = px + req.dx, py + req.dy
            local dest = tile_at(nx, ny)
            local key_ent = entities["key"]
            local guard_ent = entities["guard"]

            if guard_ent and guard_ent.x == nx and guard_ent.y == ny then
                px, py = -4, -2
                patches[#patches + 1] = { id = "player", x = px, y = py }
                msg = "Sentinel bumped you back to the courtyard!"
            elseif dest == "+" then
                if has_key then
                    door_open = true
                    px, py = nx, ny
                    patches[#patches + 1] = { id = "player", x = px, y = py }
                    msg = "Unlocked the gate with the brass key!"
                else
                    msg = "The gate is locked. Grab the k- key south of you first."
                end
            elseif dest == "." or dest == "/" or dest == "X" then
                px, py = nx, ny
                patches[#patches + 1] = { id = "player", x = px, y = py }
                if key_ent and key_ent.x == nx and key_ent.y == ny then
                    has_key = true
                    patches[#patches + 1] = { id = "key", remove = true }
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

    local guard = entities["guard"]
    if guard then
        local gx = guard.x + guard_dir
        local g_tile = tile_at(gx, guard.y)
        if g_tile == "#" or (gx == px and guard.y == py) then
            guard_dir = -guard_dir
            if gx == px and guard.y == py then
                msg = "The sentinel blocks the eastern passage."
            end
        else
            patches[#patches + 1] = { id = "guard", x = gx, y = guard.y }
        end
    end

    local key_tag = escaped and "ESCAPED!" or (has_key and "KEY: YES" or "KEY: NO")
    return patches, { status = key_tag, msg = msg }
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
