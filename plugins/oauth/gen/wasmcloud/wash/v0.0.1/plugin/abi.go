// Code generated by wit-bindgen-go. DO NOT EDIT.

package plugin

import (
	"github.com/cosmonic/wash/plugins/oauth/gen/wasmcloud/wash/v0.0.1/types"
	"go.bytecodealliance.org/cm"
)

func lift_Command(f0 *uint8, f1 uint32, f2 *uint8, f3 uint32, f4 *uint8, f5 uint32, f6 *cm.Tuple[string, types.CommandArgument], f7 uint32, f8 *types.CommandArgument, f9 uint32, f10 *string, f11 uint32) (v types.Command) {
	v.ID = cm.LiftString[string](f0, f1)
	v.Name = cm.LiftString[string](f2, f3)
	v.Description = cm.LiftString[string](f4, f5)
	v.Flags = cm.LiftList[cm.List[cm.Tuple[string, types.CommandArgument]]](f6, f7)
	v.Arguments = cm.LiftList[cm.List[types.CommandArgument]](f8, f9)
	v.Usage = cm.LiftList[cm.List[string]](f10, f11)
	return
}
