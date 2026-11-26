-- Tidekeep: default Phase 3 world script for turtle.
-- Combines procedural archipelago terrain, pushable driftwood bridges, Sun Gems, pickups, timed
-- status effects, melee combat, and component-driven crab AI using the host ECS API.

local SEA_LEVEL = -0.15   -- Raise toward 0.45 to flood islands; lower toward -0.20 for Pangea.
local CRAB_PERIOD = 2     -- Default steps between Reef Crab moves; 1 chases fast, 99 stays calm.
local GEMS_GOAL = 5       -- Total Sun Gems placed around the starting archipelago.
local PLAYER_MAX_HP = 6   -- Starting and maximum hit points for the explorer hero.
local FLARE_STUN_TICKS = 3 -- Duration in simulation steps that a stun effect immobilizes crabs.
local INVULN_TICKS = 2    -- Grace steps after taking damage during which crab hits bounce off.

---@class TidekeepState
---@field seed integer Active world seed provided at initialization.
---@field won boolean Whether all Sun Gems have been collected.
local state = {
    seed = 42,
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
    -- Entry format: { 2-char glyph, fg Colour, bg Colour? }.
    -- Terrain sets an explicit background for spatial mass (Blue water, Black land, BrightBlack
    -- stone); actors omit bg (nil) so Rust composites them transparently over terrain.

    -- Terrain tiles:
    ["~"] = { "≈~", "BrightCyan", "Blue" },   -- Deep ocean crest; impassable, bridgeable with crates.
    ["w"] = { "~≈", "Cyan", "Blue" },         -- Deep ocean shimmer frame alternated by world.view.
    [","] = { " ·", "BrightCyan", "Blue" },   -- Coastal shoals with sea spray; impassable water.
    ["."] = { " ·", "Yellow", "Black" },      -- Sandy beach / dry shore; walkable.
    ['"'] = { " ,", "Green", "Black" },       -- Grassy island interior turf; walkable.
    ["#"] = { "▒▒", "White", "BrightBlack" }, -- Stone ruins / sea cliffs; blocks actors and crates.
    ["="] = { "==", "BrightYellow", "Blue" }, -- Driftwood plank bridge over water; walkable.

    -- Overlaid entities (bg = nil inherits underlying terrain or bridge background):
    ["@"] = { "@ ", "BrightYellow" },         -- Explorer / player hero.
    ["B"] = { "[]", "Yellow" },               -- Driftwood crate; slides on land, bridges water.
    ["G"] = { "<>", "BrightCyan" },           -- Sun Gem facet; collectible goal item.
    ["C"] = { "><", "BrightRed" },            -- Reef Crab pincers; patrolling hazard.
    ["K"] = { "}{", "Red" },                  -- Armored Shell Crab; sturdy fearless guardian.
    ["Z"] = { "z*", "BrightMagenta" },        -- Stunned crab indicator while immobilized.
    ["+"] = { "++", "BrightGreen" },          -- Healing Tonic pickup restoring explorer health.
    ["/"] = { "/^", "BrightWhite" },          -- Coral Spear pickup boosting melee attack power.
}

--- Evaluates deterministic pseudo-elevation for an arbitrary world tile.
---@param x integer Eastward coordinate in world tiles.
---@param y integer Southward coordinate in world tiles.
---@param seed integer Deterministic world seed.
---@return number height Elevation scalar centered near zero.
local function elevation(x, y, seed)
    -- Three-octave trigonometric wave synthesis: an O(1) stateless stand-in for 2D gradient noise
    -- so any (x, y) tile evaluates independently without precomputing a grid or permutation table.
    -- All numeric coefficients below are hand-tuned heuristics rather than physical constants.

    -- 1. Seed-to-phase projection: split seed across coprime moduli 97 and 89 (8,633 unique phase
    -- pairs) with radian steps 0.13 and 0.17 (each spanning > 2*pi across its modulus range) so
    -- consecutive seeds shift X and Y phases independently instead of sliding diagonally in sync.
    local p1 = (seed % 97) * 0.13
    local p2 = ((math.floor(seed / 97)) % 89) * 0.17

    -- 2. Base island lattice (Octave 1): asymmetric spatial frequencies 0.24 and 0.28 rad/tile give
    -- wavelengths of ~26 tiles east-west and ~22 tiles north-south, fitting ~2-3 island-and-channel
    -- cycles across the viewport without square-grid alignment.
    local fx = x * 0.24 + p1
    local fy = y * 0.28 + p2

    -- Multiplying sin(fx) * cos(fy) forms an alternating 2D lattice of hills (+1) and basins (-1);
    -- adding two weaker diagonal harmonics with decaying weights (1.0, 0.45, 0.25) and
    -- non-integer ratios (1.9/1.3 and opposing 0.61*x - 0.53*y) breaks checkerboard symmetry into
    -- irregular bays, peninsulas, and narrow straits without obvious repeating seams.
    local wave = math.sin(fx) * math.cos(fy)
        + 0.45 * math.sin(fx * 1.9 + fy * 1.3 + p2)
        + 0.25 * math.cos(x * 0.61 - y * 0.53 + p1)

    -- 3. Amplitude scaling: worst-case octave sum is 1.0 + 0.45 + 0.25 = 1.70; scaling by 0.55
    -- keeps values in [-0.935, +0.935] inside [-1, 1] so SEA_LEVEL in terrain_at sweeps smoothly.
    return wave * 0.55
end

--- Determines the static terrain key beneath any coordinate before animated shimmer.
---@param x integer Eastward coordinate in world tiles.
---@param y integer Southward coordinate in world tiles.
---@return string tile Palette key representing terrain at the requested cell.
local function terrain_at(x, y)
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

--- Reports whether a live bridge entity occupies the given coordinate.
---@param x integer Eastward coordinate in world tiles.
---@param y integer Southward coordinate in world tiles.
---@param ecs any Scoped host entity-component world handle.
---@return boolean bridged True when a bridge entity sits on the cell.
local function has_bridge_at(x, y, ecs)
    for _, eid in ipairs(ecs:at(x, y)) do
        if ecs:has(eid, "bridge") then
            return true
        end
    end
    return false
end

--- Reports whether a cell supports walking due to land terrain or a placed bridge entity.
---@param x integer Eastward coordinate in world tiles.
---@param y integer Southward coordinate in world tiles.
---@param ecs any Scoped host entity-component world handle.
---@return boolean walkable True when actors or crates can occupy the coordinate.
local function is_cell_walkable(x, y, ecs)
    return is_walkable(terrain_at(x, y)) or has_bridge_at(x, y, ecs)
end

--- Reports whether a cell contains open water without a bridge entity on top.
---@param x integer Eastward coordinate in world tiles.
---@param y integer Southward coordinate in world tiles.
---@param ecs any Scoped host entity-component world handle.
---@return boolean open_water True when a crate pushed here creates a new bridge.
local function is_unbridged_water(x, y, ecs)
    return is_water(terrain_at(x, y)) and not has_bridge_at(x, y, ecs)
end

--- Returns the first solid blocking entity at a coordinate other than the ignored handle.
---@param x integer Eastward coordinate in world tiles.
---@param y integer Southward coordinate in world tiles.
---@param skip_id integer|nil Optional entity handle excluded from blocking checks.
---@param ecs any Scoped host entity-component world handle.
---@return integer|nil blocker Blocking solid entity handle if one is present.
local function find_solid_at(x, y, skip_id, ecs)
    for _, eid in ipairs(ecs:at(x, y)) do
        if eid ~= skip_id and ecs:has(eid, "solid") then
            return eid
        end
    end
    return nil
end

--- Finds the nearest walkable island coordinate outward from an anchor point.
---@param ax integer Starting search X coordinate.
---@param ay integer Starting search Y coordinate.
---@return integer x Nearest walkable X coordinate.
---@return integer y Nearest walkable Y coordinate.
local function find_nearby_land(ax, ay)
    for radius = 0, 8 do
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

--- Finds the nearest unoccupied walkable island coordinate outward from an anchor point.
---@param ax integer Starting search X coordinate.
---@param ay integer Starting search Y coordinate.
---@param ecs any Scoped host entity-component world handle.
---@return integer x Nearest free walkable X coordinate.
---@return integer y Nearest free walkable Y coordinate.
local function find_free_land(ax, ay, ecs)
    for radius = 0, 8 do
        for dy = -radius, radius do
            for dx = -radius, radius do
                local tx, ty = ax + dx, ay + dy
                if is_walkable(terrain_at(tx, ty)) and #ecs:at(tx, ty) == 0 then
                    return tx, ty
                end
            end
        end
    end
    return find_nearby_land(ax, ay)
end

--- Builds the persistent status bar string from player health, combat, inventory, and status.
---@param ecs any Scoped host entity-component world handle.
---@return string status Formatted HUD summary string.
local function hud_system(ecs)
    local player_id = ecs:find_by_name("player")
    if not player_id then
        return "GEMS 0/" .. tostring(GEMS_GOAL)
    end
    local hp = ecs:get(player_id, "hp") or { cur = PLAYER_MAX_HP, max = PLAYER_MAX_HP }
    local combat = ecs:get(player_id, "combat") or { atk = 1 }
    local inv = ecs:get(player_id, "inventory") or { gems = 0, tonics = 0, flares = 0 }
    local st = ecs:get(player_id, "status")

    local line = string.format(
        "HP %d/%d ATK %d T:%d F:%d GEMS %d/%d",
        hp.cur,
        hp.max,
        combat.atk,
        inv.tonics or 0,
        inv.flares or 0,
        inv.gems or 0,
        GEMS_GOAL
    )
    if st and st.invuln and st.invuln > 0 then
        line = line .. " [INVULN]"
    end
    if state.won then
        line = line .. " VICTORY!"
    end
    return line
end

--- Attaches or extends a stun status effect and switches crab visuals to stunned.
---@param ecs any Scoped host entity-component world handle.
---@param target_id integer Target entity handle to immobilize.
---@param ticks integer Minimum number of simulation steps the stun lasts.
local function apply_stun(ecs, target_id, ticks)
    local st = ecs:get(target_id, "status") or {}
    st.stun = math.max(st.stun or 0, ticks)
    ecs:set(target_id, "status", st)

    local ai = ecs:get(target_id, "ai")
    if ai then
        ai.state = "stunned"
        ecs:set(target_id, "ai", ai)
    end
    local sprite = ecs:get(target_id, "sprite")
    if sprite then
        sprite.tile = "Z"
        ecs:set(target_id, "sprite", sprite)
    end
end

--- Applies melee or impact damage to a hostile creature and drops loot upon defeat.
---@param ecs any Scoped host entity-component world handle.
---@param target_id integer Target creature entity handle.
---@param damage integer Positive damage subtracted from target hit points.
---@param verb string Short action verb describing the strike source.
---@return string summary Event log message describing the combat outcome.
local function damage_creature(ecs, target_id, damage, verb)
    local hp = ecs:get(target_id, "hp")
    local label = ecs:get(target_id, "name") or "crab"
    local ai = ecs:get(target_id, "ai")
    if not hp then
        return verb .. " " .. label .. "."
    end

    hp.cur = hp.cur - damage
    if hp.cur <= 0 then
        local pos = ecs:get(target_id, "pos")
        local dropped_tonic = ai and ai.base_tile == "K" and pos ~= nil
        ecs:despawn(target_id)
        if dropped_tonic then
            ecs:spawn({
                name = "drop_tonic_" .. tostring(target_id),
                pos = { x = pos.x, y = pos.y },
                sprite = { tile = "+", layer = 2 },
                pickup = { kind = "tonic", heal = 3 },
            })
            return verb .. " " .. label .. " defeated! Dropped a Healing Tonic (+)."
        end
        return verb .. " " .. label .. " defeated!"
    end

    ecs:set(target_id, "hp", hp)
    if ai and ai.cowardly and hp.cur <= 1 and ai.state ~= "stunned" then
        ai.state = "flee"
        ecs:set(target_id, "ai", ai)
    end
    return string.format("%s %s for %d dmg (%d/%d HP).", verb, label, damage, hp.cur, hp.max)
end

--- Ticks active status timers, runs regeneration healing, and prunes expired components.
---@param ecs any Scoped host entity-component world handle.
local function status_tick_system(ecs)
    for _, eid in ipairs(ecs:query("status")) do
        local st = ecs:get(eid, "status")
        if st then
            if st.regen and st.regen > 0 then
                local hp = ecs:get(eid, "hp")
                if hp and hp.cur < hp.max then
                    hp.cur = math.min(hp.max, hp.cur + 1)
                    ecs:set(eid, "hp", hp)
                end
            end

            local next_st = {}
            local remaining = 0
            for key, ticks in pairs(st) do
                if type(ticks) == "number" and ticks > 1 then
                    next_st[key] = ticks - 1
                    remaining = remaining + 1
                end
            end

            if remaining > 0 then
                ecs:set(eid, "status", next_st)
            else
                ecs:remove(eid, "status")
            end

            local ai = ecs:get(eid, "ai")
            local sprite = ecs:get(eid, "sprite")
            if ai and sprite then
                if next_st.stun and next_st.stun > 0 then
                    if sprite.tile ~= "Z" then
                        sprite.tile = "Z"
                        ecs:set(eid, "sprite", sprite)
                    end
                else
                    if ai.state == "stunned" then
                        ai.state = "patrol"
                        ecs:set(eid, "ai", ai)
                    end
                    local base_tile = ai.base_tile or "C"
                    if sprite.tile ~= base_tile then
                        sprite.tile = base_tile
                        ecs:set(eid, "sprite", sprite)
                    end
                end
            end
        end
    end
end

--- Collects any Sun Gems or supply pickups resting at the player position.
---@param player_id integer Player entity handle.
---@param x integer Player world X coordinate.
---@param y integer Player world Y coordinate.
---@param ecs any Scoped host entity-component world handle.
---@return string|nil msg Optional pickup notification for the event log.
local function collect_ground_items(player_id, x, y, ecs)
    local msg = nil
    local inv = ecs:get(player_id, "inventory") or { gems = 0, tonics = 0, flares = 0, spear = false }

    for _, eid in ipairs(ecs:at(x, y)) do
        if eid ~= player_id and not ecs:has(eid, "bridge") then
            if ecs:has(eid, "gem") then
                ecs:despawn(eid)
                inv.gems = (inv.gems or 0) + 1
                ecs:set(player_id, "inventory", inv)
                if inv.gems >= GEMS_GOAL and not state.won then
                    state.won = true
                    msg = "Collected all Sun Gems! Keep exploring or press R for a new sea."
                else
                    msg = "Found a Sun Gem! " .. tostring(inv.gems) .. "/" .. tostring(GEMS_GOAL)
                end
            elseif ecs:has(eid, "pickup") then
                local item = ecs:get(eid, "pickup")
                ecs:despawn(eid)
                if item and item.kind == "tonic" then
                    inv.tonics = (inv.tonics or 0) + 1
                    ecs:set(player_id, "inventory", inv)
                    msg = "Picked up a Healing Tonic (+). Press [E] when hurt to heal."
                elseif item and item.kind == "spear" then
                    inv.spear = true
                    ecs:set(player_id, "inventory", inv)
                    local combat = ecs:get(player_id, "combat") or { atk = 1 }
                    combat.atk = (combat.atk or 1) + (item.atk_bonus or 1)
                    ecs:set(player_id, "combat", combat)
                    msg = "Equipped Coral Spear (/^)! Attack boosted to " .. tostring(combat.atk) .. "."
                end
            end
        end
    end

    return msg
end

--- Processes player movement, melee bumps, crate pushing and slamming, pickups, and items.
---@param requests table[] Player action requests queued for this step.
---@param ecs any Scoped host entity-component world handle.
---@return string|nil msg Optional event log message emitted by player actions.
local function player_intent_system(requests, ecs)
    local msg = nil

    for _, req in ipairs(requests) do
        if type(req.id) == "number" and ecs:alive(req.id) then
            if req.action == "move" and (req.dx ~= 0 or req.dy ~= 0) then
                local cur = ecs:get(req.id, "pos")
                local px = cur and cur.x or req.x
                local py = cur and cur.y or req.y
                local nx, ny = px + req.dx, py + req.dy
                local blocker = find_solid_at(nx, ny, req.id, ecs)

                if blocker and ecs:has(blocker, "ai") then
                    local combat = ecs:get(req.id, "combat") or { atk = 1 }
                    msg = damage_creature(ecs, blocker, combat.atk or 1, "Struck")
                elseif blocker and ecs:has(blocker, "crate") then
                    local bx, by = nx + req.dx, ny + req.dy
                    local behind_solid = find_solid_at(bx, by, blocker, ecs)
                    if behind_solid and ecs:has(behind_solid, "ai") then
                        apply_stun(ecs, behind_solid, FLARE_STUN_TICKS)
                        msg = damage_creature(ecs, behind_solid, 1, "Crate slammed")
                    elseif behind_solid then
                        msg = "Blocked! Something solid sits behind that crate."
                    elseif is_unbridged_water(bx, by, ecs) then
                        ecs:despawn(blocker)
                        ecs:spawn({
                            name = "bridge_" .. tostring(bx) .. "_" .. tostring(by),
                            pos = { x = bx, y = by },
                            sprite = { tile = "=", layer = 1 },
                            bridge = true,
                        })
                        ecs:set(req.id, "pos", { x = nx, y = ny })
                        msg = "Splash! Driftwood crate formed a bridge."
                    elseif is_cell_walkable(bx, by, ecs) then
                        ecs:set(blocker, "pos", { x = bx, y = by })
                        ecs:set(req.id, "pos", { x = nx, y = ny })
                        local pick_msg = collect_ground_items(req.id, nx, ny, ecs)
                        msg = pick_msg or "Shoved driftwood crate."
                    else
                        msg = "Stone wall stops the crate."
                    end
                elseif not blocker then
                    if is_cell_walkable(nx, ny, ecs) then
                        ecs:set(req.id, "pos", { x = nx, y = ny })
                        local pick_msg = collect_ground_items(req.id, nx, ny, ecs)
                        if pick_msg then
                            msg = pick_msg
                        end
                    elseif is_unbridged_water(nx, ny, ecs) then
                        msg = "Deep tide blocks you. Push a [] crate into water to bridge!"
                    else
                        msg = "Ancient stone ruins block the way."
                    end
                end
            elseif req.action == "act" then
                local hp = ecs:get(req.id, "hp") or { cur = PLAYER_MAX_HP, max = PLAYER_MAX_HP }
                local inv = ecs:get(req.id, "inventory") or { gems = 0, tonics = 0, flares = 0 }
                if hp.cur < hp.max and (inv.tonics or 0) > 0 then
                    inv.tonics = inv.tonics - 1
                    hp.cur = math.min(hp.max, hp.cur + 3)
                    ecs:set(req.id, "inventory", inv)
                    ecs:set(req.id, "hp", hp)
                    local st = ecs:get(req.id, "status") or {}
                    st.regen = 1
                    ecs:set(req.id, "status", st)
                    msg = string.format("Drank Healing Tonic (+). Restored to %d/%d HP.", hp.cur, hp.max)
                elseif (inv.flares or 0) > 0 then
                    local cur = ecs:get(req.id, "pos") or { x = 0, y = 0 }
                    local stunned_count = 0
                    for _, cid in ipairs(ecs:query("pos", "ai")) do
                        local cpos = ecs:get(cid, "pos")
                        if cpos then
                            local dist = math.abs(cpos.x - cur.x) + math.abs(cpos.y - cur.y)
                            if dist <= 2 then
                                apply_stun(ecs, cid, FLARE_STUN_TICKS)
                                stunned_count = stunned_count + 1
                            end
                        end
                    end
                    if stunned_count > 0 then
                        inv.flares = inv.flares - 1
                        ecs:set(req.id, "inventory", inv)
                        msg = string.format("Salt Flare blinded %d nearby crab(s) for %d turns!", stunned_count, FLARE_STUN_TICKS)
                    elseif hp.cur < hp.max then
                        msg = "Hurt and out of tonics! Find a (+) Healing Tonic on the islands."
                    else
                        msg = "Healthy and clear; no crabs within Salt Flare range."
                    end
                elseif hp.cur < hp.max then
                    msg = "Out of tonics and flares! Gather (+) supplies or keep moving."
                else
                    msg = "Tidekeep awaits all five Sun Gems. Watch out for Reef Crabs!"
                end
            end
        end
    end

    return msg
end

--- Updates crab AI finite-state transitions, movement, fleeing, and player attacks.
---@param t integer Current simulation step counter.
---@param ecs any Scoped host entity-component world handle.
---@return string|nil msg Optional combat or ambush notice for the event log.
local function ai_system(t, ecs)
    local msg = nil
    local player_id = ecs:find_by_name("player")
    local player_pos = player_id and ecs:get(player_id, "pos") or nil
    local target_x = player_pos and player_pos.x or 0
    local target_y = player_pos and player_pos.y or 0

    for _, cid in ipairs(ecs:query("pos", "ai")) do
        local cpos = ecs:get(cid, "pos")
        local ai = ecs:get(cid, "ai")
        if cpos and ai then
            local st = ecs:get(cid, "status")
            local sprite = ecs:get(cid, "sprite")
            if st and st.stun and st.stun > 0 then
                if ai.state ~= "stunned" then
                    ai.state = "stunned"
                    ecs:set(cid, "ai", ai)
                end
                if sprite and sprite.tile ~= "Z" then
                    sprite.tile = "Z"
                    ecs:set(cid, "sprite", sprite)
                end
            else
                local cx, cy = cpos.x, cpos.y
                local dx_to = target_x - cx
                local dy_to = target_y - cy
                local dist = math.abs(dx_to) + math.abs(dy_to)
                local hp = ecs:get(cid, "hp")
                local aggro = ai.aggro or 7

                local next_state = "patrol"
                if dist <= aggro and ai.cowardly and hp and hp.cur <= 1 then
                    next_state = "flee"
                elseif dist > 0 and dist <= aggro then
                    next_state = "chase"
                end

                if ai.state ~= next_state then
                    ai.state = next_state
                    ecs:set(cid, "ai", ai)
                end
                local base_tile = ai.base_tile or "C"
                if sprite and sprite.tile ~= base_tile then
                    sprite.tile = base_tile
                    ecs:set(cid, "sprite", sprite)
                end

                local period = ai.period or CRAB_PERIOD
                if period > 0 and (t % period == 0) then
                    local cand = {}
                    if next_state == "chase" then
                        local sx = dx_to > 0 and 1 or (dx_to < 0 and -1 or 0)
                        local sy = dy_to > 0 and 1 or (dy_to < 0 and -1 or 0)
                        if math.abs(dx_to) >= math.abs(dy_to) then
                            if sx ~= 0 then cand[#cand + 1] = { sx, 0 } end
                            if sy ~= 0 then cand[#cand + 1] = { 0, sy } end
                        else
                            if sy ~= 0 then cand[#cand + 1] = { 0, sy } end
                            if sx ~= 0 then cand[#cand + 1] = { sx, 0 } end
                        end
                    elseif next_state == "flee" then
                        local away_x = dx_to > 0 and -1 or (dx_to < 0 and 1 or 0)
                        local away_y = dy_to > 0 and -1 or (dy_to < 0 and 1 or 0)
                        if math.abs(dx_to) >= math.abs(dy_to) then
                            if away_x ~= 0 then cand[#cand + 1] = { away_x, 0 } end
                            if away_y ~= 0 then cand[#cand + 1] = { 0, away_y } end
                            cand[#cand + 1] = { 0, 1 }
                            cand[#cand + 1] = { 0, -1 }
                        else
                            if away_y ~= 0 then cand[#cand + 1] = { 0, away_y } end
                            if away_x ~= 0 then cand[#cand + 1] = { away_x, 0 } end
                            cand[#cand + 1] = { 1, 0 }
                            cand[#cand + 1] = { -1, 0 }
                        end
                    else
                        local hx = ai.home_x or cx
                        local hy = ai.home_y or cy
                        if math.abs(cx - hx) + math.abs(cy - hy) > 5 then
                            local sx = hx > cx and 1 or (hx < cx and -1 or 0)
                            local sy = hy > cy and 1 or (hy < cy and -1 or 0)
                            if sx ~= 0 then cand[#cand + 1] = { sx, 0 } end
                            if sy ~= 0 then cand[#cand + 1] = { 0, sy } end
                        else
                            local dirs = { { 1, 0 }, { -1, 0 }, { 0, 1 }, { 0, -1 } }
                            cand[#cand + 1] = dirs[math.random(1, 4)]
                        end
                    end

                    for _, step_dir in ipairs(cand) do
                        local nx, ny = cx + step_dir[1], cy + step_dir[2]
                        local hit = find_solid_at(nx, ny, cid, ecs)
                        if next_state == "chase" and hit and player_id and hit == player_id then
                            local p_status = ecs:get(player_id, "status")
                            if p_status and p_status.invuln and p_status.invuln > 0 then
                                msg = "Invulnerable! Shrugged off a crab pinch."
                            else
                                local combat = ecs:get(cid, "combat") or { atk = 1 }
                                local atk = combat.atk or 1
                                local php = ecs:get(player_id, "hp") or { cur = PLAYER_MAX_HP, max = PLAYER_MAX_HP }
                                php.cur = php.cur - atk
                                local next_p_status = p_status or {}
                                next_p_status.invuln = INVULN_TICKS
                                ecs:set(player_id, "status", next_p_status)

                                if php.cur <= 0 then
                                    php.cur = php.max
                                    ecs:set(player_id, "hp", php)
                                    ecs:set(player_id, "pos", { x = 0, y = 0 })
                                    msg = "Knocked out by a crab! Washed ashore at Tidekeep camp."
                                else
                                    ecs:set(player_id, "hp", php)
                                    msg = string.format(
                                        "Pinched by %s for %d dmg! (%d/%d HP)",
                                        ecs:get(cid, "name") or "crab",
                                        atk,
                                        php.cur,
                                        php.max
                                    )
                                end
                            end
                            break
                        elseif not hit and is_cell_walkable(nx, ny, ecs) then
                            ecs:set(cid, "pos", { x = nx, y = ny })
                            break
                        end
                    end
                end
            end
        end
    end

    return msg
end

--- Initializes world state and spawns the hero, crates, gems, supplies, and patrolling crabs.
---@param seed integer Deterministic random seed selected by the host engine.
---@param ecs any Scoped host entity-component world handle.
---@return table hud Initial status text and welcome message.
function world.init(seed, ecs)
    state.seed = seed
    state.won = false

    ecs:spawn({
        name = "player",
        pos = { x = 0, y = 0 },
        sprite = { tile = "@", layer = 10 },
        player = true,
        solid = true,
        hp = { cur = PLAYER_MAX_HP, max = PLAYER_MAX_HP },
        combat = { atk = 1 },
        inventory = { gems = 0, tonics = 1, flares = 2, spear = false },
    })

    ecs:spawn({ name = "crate1", pos = { x = 2, y = 0 }, sprite = { tile = "B", layer = 4 }, solid = true, crate = true })
    ecs:spawn({ name = "crate2", pos = { x = 0, y = 2 }, sprite = { tile = "B", layer = 4 }, solid = true, crate = true })
    ecs:spawn({ name = "crate3", pos = { x = -2, y = 1 }, sprite = { tile = "B", layer = 4 }, solid = true, crate = true })

    ecs:spawn({ name = "gem1", pos = { x = -1, y = -1 }, sprite = { tile = "G", layer = 2 }, gem = true })
    ecs:spawn({ name = "tonic1", pos = { x = 1, y = -1 }, sprite = { tile = "+", layer = 2 }, pickup = { kind = "tonic", heal = 3 } })
    ecs:spawn({ name = "spear1", pos = { x = 5, y = 0 }, sprite = { tile = "/", layer = 2 }, pickup = { kind = "spear", atk_bonus = 1 } })

    local gx2, gy2 = find_free_land(6, 0, ecs)
    ecs:spawn({ name = "gem2", pos = { x = gx2, y = gy2 }, sprite = { tile = "G", layer = 2 }, gem = true })

    local gx3, gy3 = find_free_land(0, 6, ecs)
    ecs:spawn({ name = "gem3", pos = { x = gx3, y = gy3 }, sprite = { tile = "G", layer = 2 }, gem = true })

    local gx4, gy4 = find_free_land(-7, -5, ecs)
    ecs:spawn({ name = "gem4", pos = { x = gx4, y = gy4 }, sprite = { tile = "G", layer = 2 }, gem = true })

    local gx5, gy5 = find_free_land(8, 7, ecs)
    ecs:spawn({ name = "gem5", pos = { x = gx5, y = gy5 }, sprite = { tile = "G", layer = 2 }, gem = true })

    local tx2, ty2 = find_free_land(-5, -4, ecs)
    ecs:spawn({ name = "tonic2", pos = { x = tx2, y = ty2 }, sprite = { tile = "+", layer = 2 }, pickup = { kind = "tonic", heal = 3 } })

    local cx1, cy1 = find_free_land(7, -2, ecs)
    ecs:spawn({
        name = "crab1",
        pos = { x = cx1, y = cy1 },
        sprite = { tile = "C", layer = 6 },
        solid = true,
        hp = { cur = 2, max = 2 },
        combat = { atk = 1 },
        ai = {
            state = "patrol",
            period = CRAB_PERIOD,
            aggro = 7,
            home_x = cx1,
            home_y = cy1,
            cowardly = true,
            base_tile = "C",
        },
    })

    local cx2, cy2 = find_free_land(-6, 5, ecs)
    ecs:spawn({
        name = "crab2",
        pos = { x = cx2, y = cy2 },
        sprite = { tile = "C", layer = 6 },
        solid = true,
        hp = { cur = 2, max = 2 },
        combat = { atk = 1 },
        ai = {
            state = "patrol",
            period = CRAB_PERIOD,
            aggro = 7,
            home_x = cx2,
            home_y = cy2,
            cowardly = true,
            base_tile = "C",
        },
    })

    local sx, sy = find_free_land(4, 6, ecs)
    ecs:spawn({
        name = "shellcrab1",
        pos = { x = sx, y = sy },
        sprite = { tile = "K", layer = 6 },
        solid = true,
        hp = { cur = 3, max = 3 },
        combat = { atk = 2 },
        ai = {
            state = "patrol",
            period = CRAB_PERIOD + 1,
            aggro = 6,
            home_x = sx,
            home_y = sy,
            cowardly = false,
            base_tile = "K",
        },
    })

    return {
        status = hud_system(ecs),
        msg = "Push [] into water to bridge; bump crabs to fight; [E] tonic/flare; gather <>!",
    }
end

--- Runs the ordered simulation pipeline for status timers, player input, and crab behaviors.
---@param t integer Current one-based simulation step counter.
---@param requests table[] Player action requests queued for this step.
---@param ecs any Scoped host entity-component world handle.
---@return table hud Updated status bar and event log message.
function world.step(t, requests, ecs)
    status_tick_system(ecs)
    local player_msg = player_intent_system(requests, ecs)
    local ai_msg = ai_system(t, ecs)
    return {
        status = hud_system(ecs),
        msg = ai_msg or player_msg,
    }
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
