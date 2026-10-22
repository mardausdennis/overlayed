import type { DirectionLR, OpacityTarget } from "@/config";
import { useConfigValue } from "@/hooks/use-config-value";
import type { OverlayedUser } from "../types";
import { HeadphonesOff } from "./icons/headphones-off";
import { MicOff } from "./icons/mic-off";
import { cn } from "@/utils/tw";
import { useLayoutEffect, useRef, useState } from "react";

export const User = ({
  item,
  alignDirection,
  opacity,
  opacityTarget,
  userScale,
  anchored,
}: {
  item: OverlayedUser;
  alignDirection: DirectionLR;
  opacity: number;
  opacityTarget: OpacityTarget;
  userScale: number;
  anchored: boolean;
}) => {
  const { id, selfMuted, selfDeafened, talking, muted, deafened, avatarHash } = item;

  const avatarUrl = avatarHash ? `https://cdn.discordapp.com/avatars/${id}/${avatarHash}.jpg` : "/img/default.png";

  const talkingClass = talking ? "border-green-500" : "border-zinc-800";
  const opacityStyle = talking ? "100%" : `${opacity}%`;
  const showName = alignDirection !== "center";
  const pillBackground = `rgba(0, 0, 0, ${0.55 * (opacityTarget === "username-box" ? opacity / 100 : 1)})`;
  const scaleFactor = (userScale ?? 100) / 100;
  const rowRef = useRef<HTMLDivElement>(null);
  const [naturalSize, setNaturalSize] = useState<{ width: number; height: number } | null>(null);

  useLayoutEffect(() => {
    const row = rowRef.current;
    if (!anchored || !row) return;
    const measure = () => {
      const width = row.offsetWidth;
      const height = row.offsetHeight;
      setNaturalSize(previous =>
        previous?.width === width && previous.height === height ? previous : { width, height }
      );
    };
    const observer = new ResizeObserver(measure);
    // Styles can change only padding while the content box stays the same.
    observer.observe(row, { box: "border-box" });
    measure();
    return () => observer.disconnect();
  }, [anchored]);
  let transformOrigin = "center center";
  if (alignDirection === "left") {
    transformOrigin = "left center";
  } else if (alignDirection === "right") {
    transformOrigin = "right center";
  }

  const { value: maxUsernameLength } = useConfigValue("maxUsernameLength");

  const displayName = (() => {
    const name = item.username ?? "";
    if (!maxUsernameLength || name.length <= maxUsernameLength) return name;
    return name.slice(0, Math.max(0, maxUsernameLength)) + "…";
  })();

  function renderCheeseMicIcon() {
    let icon = null;

    if (selfMuted) {
      icon = <MicOff className={muted ? "fill-red-600" : "fill-neutral-400"} />;
    }

    if (selfDeafened) {
      icon = <HeadphonesOff className={deafened ? "fill-red-600" : "fill-neutral-400"} />;
    }

    if (muted) {
      icon = <MicOff className="fill-red-600" />;
    }

    if (deafened) {
      icon = <HeadphonesOff className="fill-red-600" />;
    }

    const anyState = selfMuted || selfDeafened || muted || deafened;

    return (
      <div
        className={cn(
          "absolute left-3 -bottom-2 pr-1 py-0.5 min-w-6 h-6 rounded-full",
          anyState ? "bg-black/80" : "bg-transparent",
          alignDirection === "center" ? "flex" : anchored ? "hidden" : "md:hidden"
        )}
      >
        {icon}
      </div>
    );
  }

  const row = (
    <div
      ref={rowRef}
      data-placement-row={anchored ? "" : undefined}
      className={cn(
        "flex gap-2 py-1 p-2 justify-start items-center transition-opacity",
        alignDirection == "right" ? "flex-row-reverse" : "flex-row",
        anchored ? "w-max shrink-0" : undefined,
        anchored && alignDirection === "center" ? "pb-2" : undefined,
        showName && "px-1.5 py-1",
        showName && !anchored && "w-fit max-w-full",
        showName && !anchored && alignDirection === "right" && "ml-auto"
      )}
      style={{
        ...(opacityTarget === "all" ? { opacity: opacityStyle } : {}),
        transform: scaleFactor !== 1 ? `scale(${scaleFactor})` : undefined,
        transformOrigin:
          scaleFactor !== 1
            ? anchored
              ? `top ${alignDirection === "right" ? "right" : "left"}`
              : transformOrigin
            : undefined,
        willChange: scaleFactor !== 1 ? "transform" : undefined,
      }}
    >
      <div className={`pointer-events-none shrink-0 relative rounded-full border-2 ${talkingClass}`}>
        <img
          onError={e => {
            // @ts-expect-error need to fix this prolly
            e.target.onerror = null;
            // @ts-expect-error need to fix this prolly
            e.target.src = "/img/default.png";
          }}
          src={avatarUrl}
          alt="avatar"
          className="rounded-full w-8 h-8"
        />

        {/* This is cheese string mode */}
        {renderCheeseMicIcon()}
      </div>

      {/* This is the normal list */}
      <div
        className={cn(
          anchored
            ? "flex shrink-0 pointer-events-none items-center rounded-full p-1 pl-2 pr-2"
            : "min-w-0 md:flex hidden pointer-events-none items-center rounded-full p-1 pl-2 pr-2",
          alignDirection === "center" ? "hidden md:hidden" : undefined,
          "text-white font-semibold"
        )}
        style={{ backgroundColor: pillBackground }}
      >
        <span className="truncate text-ellipsis">{displayName}</span>
        <div className="flex shrink-0">
          {(selfMuted || muted) && <MicOff className={muted ? "fill-red-600" : "fill-current"} />}
          {(selfDeafened || deafened) && <HeadphonesOff className={deafened ? "fill-red-600" : "fill-current"} />}
        </div>
      </div>
    </div>
  );

  if (!anchored) return row;
  return (
    <div
      className={cn("shrink-0 overflow-hidden", alignDirection === "right" && "flex items-start justify-end")}
      style={
        naturalSize ? { width: naturalSize.width * scaleFactor, height: naturalSize.height * scaleFactor } : undefined
      }
    >
      {row}
    </div>
  );
};
