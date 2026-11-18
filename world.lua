-- Tidekeep: default Phase 3 world script for turtle.
-- Combines procedural archipelago terrain, pushable driftwood bridges, Sun Gems, and Reef Crabs
-- in Ultima-style hybrid pacing. Edit the constants below and save to hot-reload at t=0.

local SEA_LEVEL = 0.14   -- Raise toward 0.45 to flood islands; lower toward -0.20 for Pangea.
local CRAB_PERIOD = 2    -- Crabs move every N steps; set 1 for intense chase or 99 for peaceful.
local GEMS_GOAL = 5      -- Total Sun Gems placed around the starting archipelago.

---@class TidekeepState
---@field seed integer Active world seed provided at initialization.
---@field gems_found integer Number of Sun Gems collected so far.
---@field bridges table<string, boolean> Coordinates bridged by pushed driftwood crates.
---@field won boolean Whether all Sun Gems have been collected.
local state = {
    seed = 42,
    gems_found = 0,
    bridges = {},
    won = false,
}

--- World module exported to the host engine.
local world = {}

--- Display name rendered in the top border bar.
world.title = "Tidekeep"

--- Terminal column width per logical tile so cards and rooms look square.
world.cell_width = 2

--- Hybrid pacing steps immediately on input and ticks ambient ocean and crabs when idle.
world.pacing = { mode = "hybrid", idle_ms = 650 }

--- Visual legend pairing short terrain and actor keys with sixteen-color terminal styles.
world.palette = {
    ["~"] = { "≈~", "BrightCyan", "Blue" },
    ["w"] = { "~≈", "Cyan", "Blue" },
    [","] = { " ·", "BrightCyan", "Blue" },
    ["."] = { " ·", "Yellow", "Black" },
    ['"'] = { " ,", "Green", "Black" },
    ["#"] = { "▒▒", "White", "BrightBlack" },
    ["="] = { "==", "BrightYellow", "Blue" },
    ["@"] = { "@ ", "BrightYellow" },
    ["B"] = { "[]", "Yellow" },
    ["G"] = { "<>", "BrightCyan" },
    ["C"] = { "><", "BrightRed" },
}

--- Encodes signed integer coordinates into a sparse lookup key.
---@param x integer Eastward coordinate in world tiles.
---@param y integer Southward coordinate in world tiles.
---@return string key Stable string key representing the coordinate pair.
local function coord_key(x, y)
    return tostring(x) .. "," .. tostring(y)
end

--- Evaluates deterministic pseudo-elevation for an arbitrary world tile.
---@param x integer Eastward coordinate in world tiles.
---@param y integer Southward coordinate in world tiles.
---@param seed integer Deterministic world seed.
---@return number height Elevation scalar centered near zero.
local function elevation(x, y, seed)
    local p1 = (seed % 97) * 0.13
    local p2 = ((math.floor(seed / 97)) % 89) * 0.17
    local fx = x * 0.24 + p1
    local fy = y * 0.28 + p2
    local wave = math.sin(fx) * math.cos(fy)
        + 0.45 * math.sin(fx * 1.9 + fy * 1.3 + p2)
        + 0.25 * math.cos(x * 0.61 - y * 0.53 + p1)
    return wave * 0.55
end

--- Determines the static terrain key beneath any coordinate before animated shimmer.
---@param x integer Eastward coordinate in world tiles.
---@param y integer Southward coordinate in world tiles.
---@return string tile Palette key representing terrain at the requested cell.
local function terrain_at(x, y)
    if state.bridges[coord_key(x, y)] then
        return "="
    end

    -- Keep a small starting camp and a guaranteed nearby crate-bridge puzzle channel.
    if (x == -2 and y == -2) or (x == 2 and y == -2) then
        return "#"
    end
    if math.abs(x) <= 2 and math.abs(y) <= 2 then
        if math.abs(x) == 2 or math.abs(y) == 2 then
            return "."
        end
        return '"'
    end
    if y == 0 and (x == 3 or x == 4) then
        return "~"
    end
    if y == 0 and (x == 5 or x == 6 or x == 7) then
        return "."
    end
    if x == 0 and (y == 3 or y == 4) then
        return ","
    end
    if x == 0 and (y == 5 or y == 6) then
        return "."
    end

    local h = elevation(x, y, state.seed)
    if h < SEA_LEVEL - 0.10 then
        return "~"
    elseif h < SEA_LEVEL then
        return ","
    elseif h < SEA_LEVEL + 0.11 then
        return "."
    elseif h > SEA_LEVEL + 0.68 then
        return "#"
    else
        return '"'
    end
end

--- Reports whether a terrain key represents impassable water.
---@param tile string Palette key to classify.
---@return boolean water True when the tile is ocean or shoal water.
local function is_water(tile)
    return tile == "~" or tile == "w" or tile == ","
end

--- Reports whether actors can stand on the specified terrain tile.
---@param tile string Palette key to classify.
---@return boolean walkable True when the tile supports walking entities.
local function is_walkable(tile)
    return tile == "." or tile == '"' or tile == "="
end

--- Finds the nearest walkable island coordinate outward from an anchor point.
---@param ax integer Starting search X coordinate.
---@param ay integer Starting search Y coordinate.
---@return integer x Nearest walkable X coordinate.
---@return integer y Nearest walkable Y coordinate.
local function find_nearby_land(ax, ay)
    for radius = 0, 6 do
        for dy = -radius, radius do
            for dx = -radius, radius do
                local tx, ty = ax + dx, ay + dy
                if is_walkable(terrain_at(tx, ty)) then
                    return tx, ty
                end
            end
        end
    end
    return 0, 0
end

--- Formats the persistent status bar string summarizing collected Sun Gems.
---@return string status Formatted gem progress and victory notice when complete.
local function format_status()
    if state.won then
        return "GEMS " .. tostring(state.gems_found) .. "/" .. tostring(GEMS_GOAL) .. " VICTORY!"
    end
    return "GEMS " .. tostring(state.gems_found) .. "/" .. tostring(GEMS_GOAL)
end

--- Initializes world state, starting hero, driftwood crates, Sun Gems, and Reef Crabs.
---@param seed integer Deterministic random seed selected by the host engine.
---@return table[] entities Initial actor list registered with the host.
---@return table hud Initial status text and welcome message.
function world.init(seed)
    state.seed = seed
    state.gems_found = 0
    state.bridges = {}
    state.won = false

    local gx2, gy2 = find_nearby_land(6, 0)
    local gx3, gy3 = find_nearby_land(0, 6)
    local gx4, gy4 = find_nearby_land(-7, -5)
    local gx5, gy5 = find_nearby_land(8, 7)

    local cx1, cy1 = find_nearby_land(7, -2)
    local cx2, cy2 = find_nearby_land(-6, 5)

    local entities = {
        { id = "player", x = 0, y = 0, tile = "@", player = true },
        { id = "crate1", x = 2, y = 0, tile = "B" },
        { id = "crate2", x = 0, y = 2, tile = "B" },
        { id = "crate3", x = -2, y = 1, tile = "B" },
        { id = "gem1", x = -1, y = -1, tile = "G" },
        { id = "gem2", x = gx2, y = gy2, tile = "G" },
        { id = "gem3", x = gx3, y = gy3, tile = "G" },
        { id = "gem4", x = gx4, y = gy4, tile = "G" },
        { id = "gem5", x = gx5, y = gy5, tile = "G" },
        { id = "crab1", x = cx1, y = cy1, tile = "C" },
        { id = "crab2", x = cx2, y = cy2, tile = "C" },
    }

    local hud = {
        status = format_status(),
        msg = "Push [] crates into water to build == bridges. Gather all Sun Gems <>!",
    }
    return entities, hud
end

--- Advances player actions, crate bridging physics, gem pickups, and Reef Crab patrols.
---@param t integer Current one-based simulation step counter.
---@param requests table[] Player action requests queued for this step.
---@param entities table<string, table> Read-only entity snapshot keyed by identifier.
---@return table[] patches Incremental position, spawn, or removal updates for the host.
---@return table hud Optional status and message updates rendered in the HUD.
function world.step(t, requests, entities)
    local patches = {}
    local msg = nil

    -- Track pending coordinates locally so crab AI reacts to the player's latest move.
    local pos_x = {}
    local pos_y = {}
    local removed = {}
    for id, ent in pairs(entities) do
        pos_x[id] = ent.x
        pos_y[id] = ent.y
    end

    local function live_at(tx, ty, skip_id)
        for id, ent in pairs(entities) do
            if id ~= skip_id and not removed[id] and pos_x[id] == tx and pos_y[id] == ty then
                return ent
            end
        end
        return nil
    end

    for _, req in ipairs(requests) do
        if req.action == "move" and (req.dx ~= 0 or req.dy ~= 0) then
            local px = pos_x[req.id] or req.x
            local py = pos_y[req.id] or req.y
            local nx, ny = px + req.dx, py + req.dy
            local blocker = live_at(nx, ny, req.id)

            if blocker and blocker.tile == "G" then
                removed[blocker.id] = true
                patches[#patches + 1] = { id = blocker.id, remove = true }
                pos_x[req.id], pos_y[req.id] = nx, ny
                patches[#patches + 1] = { id = req.id, x = nx, y = ny }
                state.gems_found = state.gems_found + 1
                if state.gems_found >= GEMS_GOAL and not state.won then
                    state.won = true
                    msg = "Collected all Sun Gems! Keep exploring or press R for a new sea."
                else
                    msg = "Found a Sun Gem! " .. tostring(state.gems_found) .. "/" .. tostring(GEMS_GOAL)
                end
            elseif blocker and blocker.tile == "C" then
                pos_x[req.id], pos_y[req.id] = 0, 0
                patches[#patches + 1] = { id = req.id, x = 0, y = 0 }
                msg = "Pinched by a Reef Crab! Retreated to Tidekeep camp."
            elseif blocker and blocker.tile == "B" then
                local bx, by = nx + req.dx, ny + req.dy
                local behind_ent = live_at(bx, by, blocker.id)
                local behind_tile = terrain_at(bx, by)
                if behind_ent then
                    msg = "Blocked! Something sits behind that crate."
                elseif is_water(behind_tile) then
                    removed[blocker.id] = true
                    state.bridges[coord_key(bx, by)] = true
                    patches[#patches + 1] = { id = blocker.id, remove = true }
                    pos_x[req.id], pos_y[req.id] = nx, ny
                    patches[#patches + 1] = { id = req.id, x = nx, y = ny }
                    msg = "Splash! Driftwood crate formed a bridge."
                elseif is_walkable(behind_tile) then
                    pos_x[blocker.id], pos_y[blocker.id] = bx, by
                    pos_x[req.id], pos_y[req.id] = nx, ny
                    patches[#patches + 1] = { id = blocker.id, x = bx, y = by }
                    patches[#patches + 1] = { id = req.id, x = nx, y = ny }
                    msg = "Shoved driftwood crate."
                else
                    msg = "Stone wall stops the crate."
                end
            elseif not blocker then
                local dest_tile = terrain_at(nx, ny)
                if is_walkable(dest_tile) then
                    pos_x[req.id], pos_y[req.id] = nx, ny
                    patches[#patches + 1] = { id = req.id, x = nx, y = ny }
                elseif is_water(dest_tile) then
                    msg = "Deep tide blocks you. Push a [] crate into water to bridge!"
                else
                    msg = "Ancient stone ruins block the way."
                end
            end
        elseif req.action == "act" then
            msg = "Tidekeep camp awaits all five Sun Gems."
        end
    end

    -- Move Reef Crabs on their configured beat, chasing nearby explorers or patrolling shore.
    if CRAB_PERIOD > 0 and (t % CRAB_PERIOD == 0) then
        local target_x = pos_x["player"] or 0
        local target_y = pos_y["player"] or 0

        for id, ent in pairs(entities) do
            if ent.tile == "C" and not removed[id] then
                local cx, cy = pos_x[id], pos_y[id]
                local dx_to = target_x - cx
                local dy_to = target_y - cy
                local dist = math.abs(dx_to) + math.abs(dy_to)
                local cand = {}

                if dist > 0 and dist <= 7 then
                    local sx = dx_to > 0 and 1 or (dx_to < 0 and -1 or 0)
                    local sy = dy_to > 0 and 1 or (dy_to < 0 and -1 or 0)
                    if math.abs(dx_to) >= math.abs(dy_to) then
                        if sx ~= 0 then cand[#cand + 1] = { sx, 0 } end
                        if sy ~= 0 then cand[#cand + 1] = { 0, sy } end
                    else
                        if sy ~= 0 then cand[#cand + 1] = { 0, sy } end
                        if sx ~= 0 then cand[#cand + 1] = { sx, 0 } end
                    end
                else
                    local dirs = { { 1, 0 }, { -1, 0 }, { 0, 1 }, { 0, -1 } }
                    local pick = dirs[math.random(1, 4)]
                    cand[#cand + 1] = pick
                end

                for _, step_dir in ipairs(cand) do
                    local nx, ny = cx + step_dir[1], cy + step_dir[2]
                    local hit = live_at(nx, ny, id)
                    if hit and hit.id == "player" then
                        pos_x["player"], pos_y["player"] = 0, 0
                        patches[#patches + 1] = { id = "player", x = 0, y = 0 }
                        msg = "A Reef Crab ambushed you! Back to camp."
                        break
                    elseif not hit and is_walkable(terrain_at(nx, ny)) then
                        pos_x[id], pos_y[id] = nx, ny
                        patches[#patches + 1] = { id = id, x = nx, y = ny }
                        break
                    end
                end
            end
        end
    end

    return patches, { status = format_status(), msg = msg }
end

--- Builds the visible row-major tile array covering inclusive world bounds.
---@param start_pos table Top-left world corner containing integer `x` and `y` fields.
---@param end_pos table Bottom-right world corner containing integer `x` and `y` fields.
---@param t integer Current simulation step used for subtle ocean wave shimmer.
---@return string[][] rows Two-dimensional array indexed by row then column holding palette keys.
function world.view(start_pos, end_pos, t)
    local rows = {}
    local ry = 1
    for y = start_pos.y, end_pos.y do
        local row = {}
        local rx = 1
        for x = start_pos.x, end_pos.x do
            local key = terrain_at(x, y)
            if key == "~" and ((x + y + t) % 4 == 0) then
                key = "w"
            end
            row[rx] = key
            rx = rx + 1
        end
        rows[ry] = row
        ry = ry + 1
    end
    return rows
end

return world
